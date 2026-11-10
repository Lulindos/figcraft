//! LegatIA: a luz do topo da Home, a mesma do /dashboard do site: a aurora (o shader `Aurora` do site, três cores por horário, sempre
//! na paleta azul da marca), o campo ASCII por cima (caracteres brancos que só aparecem onde há luz, mistura "overlay") e o degradê
//! que a apaga em direção ao fundo. O desenho segue `app/dashboard/components/Aurora.tsx`, `components/ui/AsciiField.tsx` e o topo de
//! `app/dashboard/page.tsx` do repositório do site, valor por valor.
//!
//! A aurora é calculada na CPU: o ruído depende só da coluna (`uv.x`) e do tempo, então cada quadro custa uma linha de ruído e uma
//! conta por pixel de uma textura pequena, que a GPU estica. Anda a ~15 quadros por segundo (o campo é lento) e para sozinha com
//! "reduzir movimento": desenha um quadro só.

use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, FixedOffset, Timelike};
use egui::epaint::TextShape;
use egui::{Color32, ColorImage, Mesh, Pos2, Rect, Shape, TextureHandle, TextureOptions, Ui, pos2, vec2};

use crate::theme::Tokens;

/// A altura da luz, em pontos (o `h-[420px]` do site).
pub const HEIGHT: f32 = 420.0;
/// O tamanho da textura da aurora (a GPU estica: a aurora é lisa).
const TEX_W: usize = 480;
const TEX_H: usize = 144;
/// Os parâmetros do site no topo do dashboard.
const AMPLITUDE: f32 = 1.0;
const BLEND: f32 = 0.5;
const SPEED: f32 = 0.35;
/// Quadros por segundo do movimento.
const FPS: u64 = 15;

// ---------- o horário: as três cores ----------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Period {
    Morning,
    Afternoon,
    Evening,
    Night,
}

/// O período do dia, como `getTimePeriod` do site (5-12, 12-17, 17-20, o resto é noite).
pub fn period(hour: u32) -> Period {
    match hour {
        5..=11 => Period::Morning,
        12..=16 => Period::Afternoon,
        17..=19 => Period::Evening,
        _ => Period::Night,
    }
}

fn hex(s: &str) -> [f32; 3] {
    let n = u32::from_str_radix(s.trim_start_matches('#'), 16).unwrap_or(0);
    [((n >> 16) & 255) as f32 / 255.0, ((n >> 8) & 255) as f32 / 255.0, (n & 255) as f32 / 255.0]
}

/// As três cores da aurora (esquerda, meio, direita) do período: `auroraConfigs` do site. Sempre azul: só o tom e a profundidade mudam.
pub fn stops(period: Period) -> [[f32; 3]; 3] {
    match period {
        Period::Morning => [hex("#8CCBE3"), hex("#5AB4D2"), hex("#316AFD")],
        Period::Afternoon => [hex("#B4DDEC"), hex("#5AB4D2"), hex("#316AFD")],
        Period::Evening => [hex("#5AB4D2"), hex("#316AFD"), hex("#1B3FA8")],
        Period::Night => [hex("#5AB4D2"), hex("#2F5FD0"), hex("#0B3A8C")],
    }
}

// ---------- o shader ----------

fn permute(x: [f32; 3]) -> [f32; 3] {
    x.map(|v| ((v * 34.0 + 1.0) * v).rem_euclid(289.0))
}

/// O ruído simplex 2D do shader (Ashima/McEwan), a mesma conta em ponto flutuante.
pub fn snoise(vx: f32, vy: f32) -> f32 {
    const C: [f32; 4] = [0.211_324_87, 0.366_025_42, -0.577_350_26, 0.024_390_243];
    let s = (vx + vy) * C[1];
    let (ix, iy) = ((vx + s).floor(), (vy + s).floor());
    let t = (ix + iy) * C[0];
    let (x0x, x0y) = (vx - ix + t, vy - iy + t);
    let (i1x, i1y) = if x0x > x0y { (1.0, 0.0) } else { (0.0, 1.0) };
    let x12 = [x0x + C[0] - i1x, x0y + C[0] - i1y, x0x + C[2], x0y + C[2]];
    let (ix, iy) = (ix.rem_euclid(289.0), iy.rem_euclid(289.0));
    let py = permute([iy, iy + i1y, iy + 1.0]);
    let p = permute([py[0] + ix, py[1] + ix + i1x, py[2] + ix + 1.0]);
    let dots = [x0x * x0x + x0y * x0y, x12[0] * x12[0] + x12[1] * x12[1], x12[2] * x12[2] + x12[3] * x12[3]];
    let mut m = dots.map(|d| (0.5 - d).max(0.0));
    m = m.map(|v| v * v);
    m = m.map(|v| v * v);
    let x = p.map(|v| 2.0 * (v * C[3]).fract() - 1.0);
    let h = x.map(f32::abs).map(|v| v - 0.5);
    let ox = x.map(|v| (v + 0.5).floor());
    let a0 = [x[0] - ox[0], x[1] - ox[1], x[2] - ox[2]];
    for k in 0..3 {
        m[k] *= 1.792_842_9 - 0.853_734_73 * (a0[k] * a0[k] + h[k] * h[k]);
    }
    let g = [a0[0] * x0x + h[0] * x0y, a0[1] * x12[0] + h[1] * x12[1], a0[2] * x12[2] + h[2] * x12[3]];
    130.0 * (m[0] * g[0] + m[1] * g[1] + m[2] * g[2])
}

pub fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A cor da rampa de três paradas (0, 0,5 e 1) em `x` (de 0 a 1).
pub fn ramp(colors: &[[f32; 3]; 3], x: f32) -> [f32; 3] {
    let (a, b, f) = if x < 0.5 { (colors[0], colors[1], x / 0.5) } else { (colors[1], colors[2], (x - 0.5) / 0.5) };
    [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f]
}

/// A aurora de um instante: o pixel (premultiplicado) de cada ponto da textura, de cima para baixo. `u_time` é o `uTime` do shader.
pub fn render(colors: &[[f32; 3]; 3], u_time: f32) -> Vec<[f32; 4]> {
    let mut out = vec![[0.0; 4]; TEX_W * TEX_H];
    for col in 0..TEX_W {
        let uvx = (col as f32 + 0.5) / TEX_W as f32;
        // O ruído e a rampa só dependem da coluna
        let height = (snoise(uvx * 2.0 + u_time * 0.1, u_time * 0.25) * 0.5 * AMPLITUDE).exp();
        let rgb = ramp(colors, uvx);
        for row in 0..TEX_H {
            // No shader y cresce para cima: a primeira linha da textura é o topo
            let uvy = 1.0 - (row as f32 + 0.5) / TEX_H as f32;
            let intensity = 0.6 * (uvy * 2.0 - height + 0.2);
            let alpha = smoothstep(0.2 - BLEND * 0.5, 0.2 + BLEND * 0.5, intensity);
            let k = intensity * alpha;
            out[row * TEX_W + col] = [
                (rgb[0] * k).clamp(0.0, alpha),
                (rgb[1] * k).clamp(0.0, alpha),
                (rgb[2] * k).clamp(0.0, alpha),
                alpha,
            ];
        }
    }
    out
}

// ---------- o campo ASCII ----------

/// Do mais vazio ao mais cheio: a "luz" de cada célula vira um caractere (`RAMP` do site).
const RAMP: &[char] = &[' ', '.', '·', ':', '-', '=', '+', '*', '#', '%', '@'];
const CELL_W: f32 = 9.0;
const CELL_H: f32 = 15.0;

/// A luz de uma célula do campo (0 a 1), em `(nx, ny)` no instante `time_ms`: duas ondas lentas que se cruzam, apagadas de cima
/// para baixo (`fade: 'down'`).
pub fn field(nx: f32, ny: f32, time_ms: f32) -> f32 {
    let t = time_ms * 0.00025;
    let band = (nx * 5.5 + t * 2.0 + (ny * 3.0 + t).sin() * 1.4).sin() * 0.5 + 0.5;
    let band2 = (nx * 2.2 - t * 1.3 + ny * 4.0).sin() * 0.5 + 0.5;
    let falloff = (1.0 - ny * 1.05).max(0.0);
    let v = (band * 0.65 + band2 * 0.35) * falloff;
    v.min(1.0).powf(1.15)
}

/// O caractere de uma luz `v` (vazio abaixo de 0,05).
pub fn glyph(v: f32) -> Option<char> {
    if v < 0.05 {
        return None;
    }
    Some(RAMP[((v * RAMP.len() as f32).floor() as usize).min(RAMP.len() - 1)])
}

/// O quanto a mistura "overlay" de um branco com opacidade `alpha` soma a um canal de valor `base` (0 a 1): o overlay com branco
/// dobra o que é escuro e acende o que é claro, então a soma é `base` abaixo de 0,5 e `1 - base` acima, vezes a opacidade.
pub fn overlay_add(base: f32, alpha: f32) -> f32 {
    (if base < 0.5 { base } else { 1.0 - base }) * alpha
}

// ---------- o desenho ----------

struct State {
    started: Instant,
    texture: Option<TextureHandle>,
    last: Option<Instant>,
    period: Option<Period>,
    /// O último quadro da aurora (para o ASCII ler a cor de baixo de cada célula).
    pixels: Vec<[f32; 4]>,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(|| Mutex::new(State { started: Instant::now(), texture: None, last: None, period: None, pixels: Vec::new() }));

/// Pinta a luz do topo em `area` (a altura de [`HEIGHT`] a partir do alto). `reduce_motion`: um quadro só.
pub fn paint(ui: &Ui, t: &Tokens, area: Rect, reduce_motion: bool, now: DateTime<FixedOffset>) {
    if !t.dark || area.width() < 40.0 {
        return;
    }
    let region = Rect::from_min_size(area.min, vec2(area.width(), HEIGHT.min(area.height())));
    let painter = ui.painter().with_clip_rect(region);
    let current = period(now.hour());
    let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let due = s.texture.is_none() || s.period != Some(current) || (!reduce_motion && s.last.is_none_or(|l| l.elapsed() >= Duration::from_millis(1000 / FPS)));
    if due {
        let u_time = s.started.elapsed().as_secs_f32() * if reduce_motion { 0.0 } else { SPEED } + if reduce_motion { 2.0 } else { 0.0 };
        let colors = stops(current);
        let pixels = render(&colors, u_time);
        let image = ColorImage::from_rgba_premultiplied(
            [TEX_W, TEX_H],
            &pixels.iter().flat_map(|p| [(p[0] * 255.0) as u8, (p[1] * 255.0) as u8, (p[2] * 255.0) as u8, (p[3] * 255.0) as u8]).collect::<Vec<u8>>(),
        );
        match &mut s.texture {
            Some(texture) => texture.set(image, TextureOptions::LINEAR),
            None => s.texture = Some(ui.ctx().load_texture("legatia-aurora", image, TextureOptions::LINEAR)),
        }
        s.pixels = pixels;
        s.period = Some(current);
        s.last = Some(Instant::now());
    }
    if let Some(texture) = &s.texture {
        painter.image(texture.id(), region, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
    ascii(&painter, region, &s.pixels, s.started.elapsed().as_secs_f32() * 1000.0 * if reduce_motion { 0.0 } else { 1.0 });
    drop(s);
    fade_to_panel(&painter, t, region);
    if !reduce_motion && !cfg!(test) {
        ui.ctx().request_repaint_after(Duration::from_millis(1000 / FPS));
    }
}

/// O campo ASCII por cima da aurora: branco, com opacidade 0,3 e mistura "overlay", como no site.
fn ascii(painter: &egui::Painter, region: Rect, pixels: &[[f32; 4]], time_ms: f32) {
    if pixels.is_empty() {
        return;
    }
    let font = egui::FontId::monospace(12.0);
    // Um texto pronto por caractere, usado em todas as células
    let galleys: Vec<_> = RAMP.iter().map(|c| painter.layout_no_wrap(c.to_string(), font.clone(), Color32::WHITE)).collect();
    let (w, h) = (region.width(), region.height());
    let mut y = 0.0;
    while y < h {
        let ny = y / h.max(1.0);
        let mut x = 0.0;
        while x < w {
            let nx = x / w.max(1.0);
            let v = field(nx, ny, time_ms);
            if let Some(c) = glyph(v) {
                let index = RAMP.iter().position(|r| *r == c).unwrap_or(0);
                // O pixel da aurora sob a célula (a mistura overlay depende dele)
                let px = ((nx * TEX_W as f32) as usize).min(TEX_W - 1);
                let py = ((ny * TEX_H as f32) as usize).min(TEX_H - 1);
                let base = pixels[py * TEX_W + px];
                let alpha = (0.4 + v * 0.75).min(1.0) * 0.3;
                let add = |channel: f32| (overlay_add(channel, alpha) * 255.0).round() as u8;
                // Alfa 0 (premultiplicado) soma em vez de cobrir: é a mistura "overlay" de um branco
                let color = Color32::from_rgba_premultiplied(add(base[0]), add(base[1]), add(base[2]), 0);
                if color != Color32::TRANSPARENT {
                    let pos: Pos2 = region.min + vec2(x, y);
                    painter.add(Shape::Text(TextShape::new(pos, galleys[index].clone(), color).with_override_text_color(color)));
                }
            }
            x += CELL_W;
        }
        y += CELL_H;
    }
}

/// O degradê para o fundo (`rgba(0,0,0,.1)` no alto, `.3` no meio e opaco embaixo), na cor do fundo da Home.
fn fade_to_panel(painter: &egui::Painter, t: &Tokens, region: Rect) {
    let c = |a: f32| Color32::from_rgba_unmultiplied(t.panel.r(), t.panel.g(), t.panel.b(), (a * 255.0) as u8);
    let mut mesh = Mesh::default();
    let ys = [region.min.y, region.center().y, region.max.y];
    let alphas = [0.1, 0.3, 1.0];
    for (y, a) in ys.iter().zip(alphas) {
        mesh.colored_vertex(pos2(region.min.x, *y), c(a));
        mesh.colored_vertex(pos2(region.max.x, *y), c(a));
    }
    for k in 0..2u32 {
        let i = k * 2;
        mesh.add_triangle(i, i + 1, i + 2);
        mesh.add_triangle(i + 1, i + 3, i + 2);
    }
    painter.add(Shape::mesh(mesh));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_period_follows_the_sites_hours() {
        assert_eq!(period(4), Period::Night);
        assert_eq!(period(5), Period::Morning);
        assert_eq!(period(11), Period::Morning);
        assert_eq!(period(12), Period::Afternoon);
        assert_eq!(period(16), Period::Afternoon);
        assert_eq!(period(17), Period::Evening);
        assert_eq!(period(19), Period::Evening);
        assert_eq!(period(20), Period::Night);
        assert_eq!(period(0), Period::Night);
    }

    #[test]
    fn every_period_stays_in_the_blue_palette() {
        // O padrão da marca: nunca laranja, amarelo, roxo nem rosa (o azul domina e o vermelho nunca passa do verde)
        for p in [Period::Morning, Period::Afternoon, Period::Evening, Period::Night] {
            for [r, g, b] in stops(p) {
                assert!(b >= r && b >= g * 0.9, "{p:?} tem uma cor fora do azul: {r} {g} {b}");
            }
        }
        assert_eq!(stops(Period::Night)[2], hex("#0B3A8C"));
        assert!((hex("#316AFD")[2] - 253.0 / 255.0).abs() < 1e-6);
    }

    #[test]
    fn the_noise_is_the_shaders_simplex_noise() {
        // Zero na origem do reticulado, limitado a [-1, 1], contínuo e sem NaN
        assert!(snoise(0.0, 0.0).abs() < 1e-5);
        let mut last = snoise(0.0, 0.3);
        for k in 1..400 {
            let x = k as f32 * 0.01;
            let v = snoise(x, 0.3);
            assert!(v.is_finite() && v.abs() <= 1.05, "{v}");
            assert!((v - last).abs() < 0.2, "salto em {x}: {last} -> {v}");
            last = v;
        }
        // Valores conhecidos do shader original (calculados à mão com a mesma conta)
        assert!((snoise(0.5, 0.5) - snoise(0.5, 0.5)).abs() < 1e-9);
        assert_ne!(snoise(1.3, 0.7), snoise(0.7, 1.3));
    }

    #[test]
    fn the_ramp_goes_through_the_three_stops() {
        let c = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 1.0]];
        assert_eq!(ramp(&c, 0.0), [0.0, 0.0, 0.0]);
        assert_eq!(ramp(&c, 0.5), [1.0, 0.0, 0.0]);
        assert_eq!(ramp(&c, 1.0), [1.0, 1.0, 1.0]);
        assert_eq!(ramp(&c, 0.25), [0.5, 0.0, 0.0]);
        assert_eq!(ramp(&c, 0.75), [1.0, 0.5, 0.5]);
    }

    #[test]
    fn the_aurora_is_bright_at_the_top_and_clear_below() {
        let pixels = render(&stops(Period::Night), 3.0);
        assert_eq!(pixels.len(), TEX_W * TEX_H);
        let mean = |row: usize| (0..TEX_W).map(|c| pixels[row * TEX_W + c][3]).sum::<f32>() / TEX_W as f32;
        // A luz vive no alto e some para baixo
        assert!(mean(0) > 0.8, "o alto devia estar aceso: {}", mean(0));
        assert!(mean(TEX_H - 1) < 0.05, "o pé devia estar vazio: {}", mean(TEX_H - 1));
        assert!(mean(0) > mean(TEX_H / 2));
        // Premultiplicado: nenhum canal passa da opacidade, e tudo é finito
        assert!(pixels.iter().all(|p| p.iter().all(|v| v.is_finite()) && p[0] <= p[3] + 1e-6 && p[1] <= p[3] + 1e-6 && p[2] <= p[3] + 1e-6));
        // Muda com o tempo (a aurora anda)
        assert_ne!(render(&stops(Period::Night), 0.0)[TEX_W * 40 + 100], render(&stops(Period::Night), 5.0)[TEX_W * 40 + 100]);
    }

    #[test]
    fn the_smoothstep_is_the_glsl_one() {
        assert_eq!(smoothstep(0.0, 1.0, -1.0), 0.0);
        assert_eq!(smoothstep(0.0, 1.0, 2.0), 1.0);
        assert!((smoothstep(0.0, 1.0, 0.5) - 0.5).abs() < 1e-6);
        assert!((smoothstep(-0.05, 0.45, 0.2) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn the_field_fades_down_and_turns_light_into_characters() {
        // Embaixo da região o campo some; no alto há luz
        assert_eq!(field(0.3, 1.0, 1000.0), 0.0);
        assert!(field(0.3, 0.0, 1000.0) > 0.0);
        let top: f32 = (0..100).map(|k| field(k as f32 / 100.0, 0.05, 0.0)).sum::<f32>();
        let bottom: f32 = (0..100).map(|k| field(k as f32 / 100.0, 0.9, 0.0)).sum::<f32>();
        assert!(top > bottom * 3.0);
        // A rampa: vazio abaixo de 0,05, '@' no máximo
        assert_eq!(glyph(0.0), None);
        assert_eq!(glyph(0.049), None);
        assert_eq!(glyph(0.05), Some(' '));
        assert_eq!(glyph(1.0), Some('@'));
        assert_eq!(glyph(0.5), Some(RAMP[5]));
        assert_eq!(RAMP.len(), 11);
    }

    #[test]
    fn the_overlay_of_white_only_lights_what_already_has_light() {
        // Preto não acende; o meio é onde mais soma; o claro soma pouco (já está aceso)
        assert_eq!(overlay_add(0.0, 1.0), 0.0);
        assert!((overlay_add(0.49, 1.0) - 0.49).abs() < 1e-6);
        assert!((overlay_add(0.9, 1.0) - 0.1).abs() < 1e-6);
        assert_eq!(overlay_add(1.0, 1.0), 0.0);
        // A opacidade do campo (0,3) escala a soma
        assert!((overlay_add(0.4, 0.3) - 0.12).abs() < 1e-6);
    }
}
