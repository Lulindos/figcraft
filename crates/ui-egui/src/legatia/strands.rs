//! LegatIA: o orbe de vidro com fios de luz do site (o `Strands` do React Bits, `components/ui/Strands.tsx`, com as cores da
//! aurora da LegatIA), desenhado aqui na CPU.
//!
//! O site roda o desenho num shader WebGL2. Aqui é a mesma conta, ponto a ponto, para um orbe de poucas dezenas de pixels:
//! cada pixel pede a luz dos fios ([`light`]), a refração do vidro ([`remap`]) e a borda ([`pixel`]). O resultado vai para
//! uma textura do egui, sem depender da placa de vídeo nem de uma passagem de desenho própria. O movimento dos fios, o pulso
//! de cada ação do agente e a agitação enquanto ele trabalha ([`Anim`]) são os do site.

use std::ops::{Add, Mul, Sub};

const PI: f32 = std::f32::consts::PI;

#[derive(Clone, Copy, Debug)]
struct V3(f32, f32, f32);

impl Add for V3 {
    type Output = V3;
    fn add(self, o: V3) -> V3 {
        V3(self.0 + o.0, self.1 + o.1, self.2 + o.2)
    }
}
impl Sub for V3 {
    type Output = V3;
    fn sub(self, o: V3) -> V3 {
        V3(self.0 - o.0, self.1 - o.1, self.2 - o.2)
    }
}
impl Mul<f32> for V3 {
    type Output = V3;
    fn mul(self, k: f32) -> V3 {
        V3(self.0 * k, self.1 * k, self.2 * k)
    }
}
impl Mul<V3> for V3 {
    type Output = V3;
    fn mul(self, o: V3) -> V3 {
        V3(self.0 * o.0, self.1 * o.1, self.2 * o.2)
    }
}

fn mix(a: V3, b: V3, t: f32) -> V3 {
    a * (1.0 - t) + b * t
}
fn mixf(a: f32, b: f32, t: f32) -> f32 {
    a * (1.0 - t) + b * t
}
fn clamp3(c: V3, lo: f32, hi: f32) -> V3 {
    V3(c.0.clamp(lo, hi), c.1.clamp(lo, hi), c.2.clamp(lo, hi))
}
fn sstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
fn step(edge: f32, x: f32) -> f32 {
    if x < edge { 0.0 } else { 1.0 }
}

/// Aproximação da função de erro (a do shader).
fn erfa(x: f32) -> f32 {
    let a = x.abs();
    let t = 1.0 / (1.0 + 0.327_591_1 * a);
    let y = 1.0 - ((((1.061_405_4 * t - 1.453_152) * t + 1.421_413_7) * t - 0.284_496_74) * t + 0.254_829_6) * t * (-a * a).exp();
    if x < 0.0 { -y } else { y }
}

fn grad4(c0: V3, c1: V3, c2: V3, c3: V3, p1: f32, p2: f32, f: f32) -> V3 {
    let c = mix(c0, c1, sstep(0.0, p1, f));
    let c = mix(c, c2, sstep(p1, p2, f));
    mix(c, c3, sstep(p2, 1.0, f))
}

fn softp(z: f32) -> f32 {
    0.03 * (1.0 + (z / 0.03).clamp(-60.0, 60.0).exp()).ln()
}

/// As cores da aurora (a do topo do painel do site) nos papéis dos fios: borda de cima e tons quentes viram azuis claros, o
/// núcleo é branco-azulado e a parte fria vai até o azul fundo da noite.
const AURORA: [&str; 7] = ["#8CCBE3", "#5AB4D2", "#B4DDEC", "#F2FAFF", "#5AB4D2", "#316AFD", "#1B3FA8"];

fn hex(h: &str) -> V3 {
    let v = u32::from_str_radix(h.trim_start_matches('#'), 16).unwrap_or(0xFFFFFF);
    V3(((v >> 16) & 255) as f32 / 255.0, ((v >> 8) & 255) as f32 / 255.0, (v & 255) as f32 / 255.0)
}

/// Tudo o que o desenho de um quadro precisa (o que o site passa ao shader como uniformes).
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    motion: [f32; 3],
    level: f32,
    lean: [f32; 2],
    intro: f32,
    pulse: f32,
}

/// A paleta e os parâmetros fixos do orbe do agente (`AgentOrb`): vidro de 1,15, três fios, nada de matiz extra.
struct Look {
    colors: [V3; 7],
}

impl Look {
    fn aurora() -> Look {
        let mut colors = [V3(0.0, 0.0, 0.0); 7];
        for (slot, h) in colors.iter_mut().zip(AURORA) {
            *slot = hex(h);
        }
        Look { colors }
    }
}

const COUNT: f32 = 3.0;
const RADIUS: f32 = 0.42 * 1.15;
const SCALE: f32 = 1.0;

fn axis(x: f32, lean: [f32; 2]) -> f32 {
    -0.2318 + lean[1] * 0.06 + (-0.2394 + lean[0] * 0.1) * x + 0.2815 * x * x + 0.3368 * x * x * x
}

fn sheet_w(tt: f32, h: f32, sig: f32, sig_b: f32, ridges: f32) -> (f32, f32) {
    let ew = 0.04;
    let mut out = [0.0f32; 2];
    for (pass, slot) in out.iter_mut().enumerate() {
        let sg = if pass == 0 { sig } else { sig_b };
        let fill = 0.5 * (erfa(tt / sg) - erfa((tt - h) / sg)) / h;
        let se = (sg * sg + 0.25 * ew * ew * h * h).sqrt();
        let de = (tt - h + 0.5 * ew * h) / se;
        let eg = (-de * de).exp() / (se * 1.772_453_9);
        let rgw = (sg * sg + 0.0025 * h * h).sqrt();
        let d1 = (tt - 0.45 * h) / rgw;
        let d2 = (tt - 0.241 * h) / rgw;
        let rid = 0.04 * ((-d1 * d1).exp() * step(1.5, ridges) + 0.7 * (-d2 * d2).exp() * step(2.5, ridges)) / (rgw * 1.772_453_9);
        *slot = 0.6 * fill + (1.6 * ew * 0.886 * eg + rid);
    }
    (out[0], out[1])
}

/// A luz dos fios no ponto `u` do orbe (a função `light` do shader); devolve também o halo.
fn light(look: &Look, f: &Frame, u: (f32, f32), fp: f32, bend: f32) -> (V3, f32) {
    let level = f.level.clamp(0.0, 1.0);
    let amp = if level < 0.65 { mixf(0.25, 1.0, sstep(0.0, 0.65, level)) } else { mixf(1.0, 1.12, (level - 0.65) / 0.35) };
    let grow = ((amp - 0.25) / 0.75).clamp(0.0, 1.25);
    let x = u.0 / SCALE;
    let uy = u.1;
    let ya = axis(x, f.lean);
    let taper = 1.0f32;
    let bx = 0.0437 + f.motion[2] + f.lean[0] * 0.08;
    let bump = (-((x - bx) / (0.6879 * taper)).powi(2)).exp();
    let h_w = 0.02 + softp(-0.12 + 0.5324 * grow * f.motion[1] * bump - 0.02);
    let k = 1.0f32;
    let cphi = (0.5 * PI + k * (x + 0.0664) + f.motion[0]).cos();
    let env_c = (0.0886 + 0.3 * (-((x - 0.0634) / 0.6873).powi(2)).exp()) * grow;
    let h_c = -env_c * (cphi / 0.6).tanh();
    let th = 0.051 * (-(cphi / 0.5).powi(2)).exp();
    let sig = ((0.005 + 0.05 * bend).powi(2) + fp * fp + th * th).sqrt();
    let sig_b = (0.0064 + fp * fp + th * th).sqrt();
    let mend = (-((x + 0.0421) / (0.4235 * taper)).abs().powf(2.2689)).exp();
    let [red, orange, amber, warm_white, cyan, blue, deep] = look.colors;
    let orange_edge = orange * V3(1.0, 0.85, 0.77);
    let cool_core = mix(warm_white, V3(1.0, 1.0, 0.7), 0.6);
    let mint = clamp3(mix(cyan, warm_white, 0.35) * V3(0.92, 1.05, 0.95), 0.0, 1.0);
    let glow = 1.0f32;
    let mut l = V3(0.0, 0.0, 0.0);
    let mut lb = V3(0.0, 0.0, 0.0);
    {
        let h = h_w.abs().max(1e-4);
        let tt = uy - ya;
        let w = sheet_w(tt, h, sig, sig_b, COUNT);
        let fr = (tt / h).clamp(0.0, 1.0);
        let g = |t: f32| grad4(warm_white, amber, orange, orange_edge, 0.2351, 0.5643, t);
        let c = g(fr);
        let c = mix(c, red, sstep(0.75, 1.0, fr) * sstep(0.196, 0.3376, h_w));
        let mean = (g(0.1) + g(0.35) + g(0.6) + g(0.9)) * 0.25;
        l = l + mix(c, mean, sstep(0.3, 1.2, sig / h)) * (w.0 * 0.2101 * mend);
        lb = lb + mix(c, mean, sstep(0.3, 1.2, sig_b / h)) * (w.1 * 0.2101 * mend);
    }
    {
        let h = h_c.abs().max(1e-4);
        let tt = (uy - ya) * if h_c >= 0.0 { 1.0 } else { -1.0 };
        let w = sheet_w(tt, h, sig, sig_b, 1.0);
        let fr = (tt / h).clamp(0.0, 1.0);
        let g = |t: f32| grad4(cool_core, cyan, blue, deep, 0.15, 0.7931, t);
        let c = g(fr);
        let mean = (g(0.1) + g(0.35) + g(0.6) + g(0.9)) * 0.25;
        l = l + mix(c, mean, sstep(0.3, 1.2, sig / h)) * (w.0 * 0.28 * mend);
        lb = lb + mix(c, mean, sstep(0.3, 1.2, sig_b / h)) * (w.1 * 0.28 * mend);
    }
    let wh = (0.0256 + env_c * env_c * cphi * cphi).sqrt();
    let dg = (uy - ya) / wh;
    let mut g = V3(1.0, 0.97, 0.92) * ((-dg * dg).exp() * 0.0315 / wh * mend * glow);
    let nx = -0.0664 - f.motion[0] / k + 0.2168;
    let bxx = (x - nx) / 0.1849;
    let byy = (uy - axis(nx, f.lean) - 0.0333) / (0.0544 * mixf(0.7, 1.0, amp.min(1.0)));
    g = g + mint * ((-bxx * bxx - byy * byy).exp() * 1.4 * glow * sstep(0.0, 0.6, amp));
    let hb = (uy - ya - 0.3 * h_w) / (0.07 + 0.45 * h_w + 0.3 * env_c);
    let intro = sstep(0.0, 0.3, f.intro);
    let halo = (-hb * hb).exp() * mend * intro;
    let total = l + lb * (0.16 * glow) + g * 1.16;
    (total * (2.4 * intro * sstep(f.intro * 1.6, f.intro * 1.6 - 0.5, x.abs())), halo)
}

/// A refração do vidro: onde, dentro do orbe, o ponto `p` (de -1 a 1) vê a luz.
fn remap(p: (f32, f32), scale: f32) -> ((f32, f32), f32, f32) {
    let len = (p.0 * p.0 + p.1 * p.1).sqrt();
    let r = len.min(0.9995);
    let bend = r.powf(5.17);
    let k = 1.0f32;
    let sc = (1.0 - 0.278 * k * (1.0 - bend)) * (1.0 - 0.632 * k * bend) / (1.0 - 0.278 * k);
    ((p.0 * sc * scale, p.1 * sc * scale), r, bend)
}

/// O pixel (`px`, `py` a partir do canto de cima à esquerda) do orbe de `size` pixels: RGBA com alfa pré-multiplicado.
fn pixel(look: &Look, f: &Frame, size: f32, px: f32, py: f32) -> [f32; 4] {
    let m = size;
    // gl_FragCoord tem o y para cima
    let p = ((px + 0.5 - 0.5 * size) / m, (0.5 * size - py - 0.5) / m);
    let appear = sstep(0.0, 0.35, f.intro);
    let radius = RADIUS * mixf(0.94, 1.0, f.intro) * (1.0 + 0.03 * f.pulse);
    let one = 1.0 / (m * radius);
    let pc = (p.0 / radius, p.1 / radius);
    let rho = (pc.0 * pc.0 + pc.1 * pc.1).sqrt();
    if rho > 1.0 + 2.0 * one {
        return [0.0; 4];
    }
    let (u, r, bend) = remap(pc, 1.0);
    // dFdx / dFdy: a diferença para o pixel vizinho
    let (ux, _, _) = remap((pc.0 + one, pc.1), 1.0);
    let (uy, _, _) = remap((pc.0, pc.1 - one), 1.0);
    let dx = ((ux.0 - u.0).powi(2) + (ux.1 - u.1).powi(2)).sqrt();
    let dy = ((uy.0 - u.0).powi(2) + (uy.1 - u.1).powi(2)).sqrt();
    let fp = 0.5 * dx.max(dy);
    let (mut lum, _halo) = light(look, f, u, fp, bend);
    let ca = 0.02 * bend;
    if ca > 0.0015 {
        let (ur, _, _) = remap(pc, 1.0 - ca);
        let (ub, _, _) = remap(pc, 1.0 + ca);
        lum.0 = light(look, f, ur, fp, bend).0 .0;
        lum.2 = light(look, f, ub, fp, bend).0 .2;
    }
    let col = V3(1.0 - (-lum.0).exp(), 1.0 - (-lum.1).exp(), 1.0 - (-lum.2).exp());
    let d = (pc.0 / rho.max(1e-6), pc.1 / rho.max(1e-6));
    let key = 0.4 * (d.0 * 0.94 - d.1 * 0.34).max(0.0).powi(6) + 0.2 * (-d.0 * 0.5 + d.1 * 0.87).max(0.0).powi(6);
    let ring = (-((r - 0.985) / 0.011).powi(2)).exp();
    let hzk = 0.25 + 0.7 * (-d.0 * 0.6 + d.1 * 0.8).max(0.0).powi(2) + (d.0 * 0.8 - d.1 * 0.6).max(0.0).powi(2);
    let haze = (-((1.0 - r) / 0.09).powi(2)).exp() * hzk * 0.13;
    let orb = V3(0.04, 0.04, 0.04) + col * (1.0 + 0.1 * bend) + V3(1.0, 1.0, 1.0) * (ring * key * (1.0 + 0.4 * f.level.clamp(0.0, 1.0))) + V3(1.0, 1.02, 1.04) * haze;
    let orb = grade(clamp3(orb, 0.0, 1.0));
    let inside = (((1.0 - rho) / one + 0.5).clamp(0.0, 1.0)) * appear;
    let rgb = orb * inside;
    let alpha = (inside * 0.92).max(rgb.0.max(rgb.1).max(rgb.2));
    [rgb.0, rgb.1, rgb.2, alpha]
}

/// A graduação de cor do shader (saturação 1,5; sem mudança de matiz).
fn grade(c: V3) -> V3 {
    let g = c.0 * 0.2126 + c.1 * 0.7152 + c.2 * 0.0722;
    clamp3(mix(V3(g, g, g), c, 1.5), 0.0, 1.0)
}

/// Desenha o orbe em `size` x `size` pixels: bytes RGBA com alfa pré-multiplicado, prontos para uma textura do egui.
pub fn render(size: usize, f: &Frame) -> Vec<u8> {
    let look = Look::aurora();
    let mut out = Vec::with_capacity(size * size * 4);
    for y in 0..size {
        for x in 0..size {
            let c = pixel(&look, f, size as f32, x as f32, y as f32);
            for v in c {
                out.push((v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
            }
        }
    }
    out
}

/// O que o desenho pede ao movimento a cada quadro.
#[derive(Clone, Copy, Debug)]
pub struct Params {
    /// O agente está trabalhando: a luz se agita como quem fala.
    pub working: bool,
    /// O painel do orbe está aberto.
    pub open: bool,
    /// Um pulso de 0 a 1 (cada ação do agente), por cima do estado.
    pub level: Option<f32>,
}

/// O movimento do orbe (o laço de quadros do `Strands`): relógio, agitação, pulso elástico e a inclinação para o ponteiro.
#[derive(Clone, Debug, Default)]
pub struct Anim {
    clock: f32,
    pace: f32,
    activity: f32,
    hover: f32,
    hover_target: f32,
    burst: f32,
    lean: [f32; 2],
    lean_target: [f32; 2],
    pulse: f32,
    pulse_v: f32,
    speech_t: f32,
    /// Quanto da entrada (0 a 1) já foi: o orbe aparece crescendo.
    intro: f32,
    intro_t: f32,
}

fn speech(t: f32) -> f32 {
    let phrase = sstep(-0.8, -0.3, 0.6 * (t * 2.7).sin() + 0.5 * (t * 1.53 + 1.2).sin());
    let syllable = ((t * 17.0 + 1.6 * (t * 5.3).sin()).sin()).max(0.0);
    let stress = 0.6 + 0.4 * (t * 3.7 + 0.7).sin();
    phrase * (0.35 + 0.65 * syllable) * (0.25 + stress)
}

impl Anim {
    /// O ponteiro está em `(x, y)` (de -1 a 1 em volta do centro do orbe), ou fora dele (`None`).
    pub fn pointer(&mut self, at: Option<(f32, f32)>) {
        match at {
            Some((x, y)) => {
                self.hover_target = (1.6 - (x * x + y * y).sqrt()).clamp(0.0, 1.0);
                self.lean_target = [x.clamp(-1.2, 1.2) * 0.8, (-y).clamp(-1.2, 1.2) * 0.8];
            }
            None => {
                self.hover_target = 0.0;
                self.lean_target = [0.0, 0.0];
            }
        }
    }

    /// Um clique: o pulso dá um tranco e a luz sobe.
    pub fn click(&mut self) {
        self.pulse_v += 5.0;
        self.burst = (self.burst + 0.9).min(1.2);
    }

    /// Avança `dt` segundos e devolve o quadro a desenhar. `speed` e `energy` são os do `AgentOrb`.
    pub fn step(&mut self, dt: f32, p: Params) -> Frame {
        let dt = dt.clamp(0.0, 0.05);
        let speed = if p.working { 1.4 } else { 1.0 };
        let energy: f32 = if p.working { 0.85 } else if p.open { 0.6 } else { 0.4 };
        self.intro_t = (self.intro_t + dt / 1.4).min(1.0);
        self.intro = 1.0 - (1.0 - self.intro_t).powi(3);
        let ease = |dt: f32, tau: f32| 1.0 - (-dt / tau).exp();
        self.hover += (self.hover_target - self.hover) * ease(dt, if self.hover_target > self.hover { 0.25 } else { 0.6 });
        for i in 0..2 {
            self.lean[i] += (self.lean_target[i] - self.lean[i]) * ease(dt, 0.35);
        }
        self.pulse_v += (-170.0 * self.pulse - 14.0 * self.pulse_v) * dt;
        self.pulse += self.pulse_v * dt;
        self.burst *= (-dt / 0.45).exp();
        if p.working {
            self.speech_t += dt;
        }
        let voice = match p.level {
            Some(l) if l > 0.02 => l.clamp(0.0, 1.0).max(if p.working { 0.5 } else { 0.0 }),
            _ if p.working => speech(self.speech_t),
            _ => 0.0,
        };
        let target = (energy.max(voice) + self.hover * 0.18 + self.burst).min(1.2);
        self.activity += (target - self.activity) * ease(dt, if target > self.activity { 0.12 } else { 0.4 });
        self.pace += (self.activity.min(1.0) - self.pace) * ease(dt, 0.8);
        self.clock += dt * speed * (0.6 + 1.6 * self.pace);
        let c = self.clock;
        let sway = 0.75 + 0.45 * self.pace;
        Frame {
            motion: [
                (0.26 * (0.53 * c).sin() + 0.1 * (1.19 * c + 1.3).sin()) * sway,
                1.0 + (0.06 * (0.71 * c).sin() + 0.035 * (1.37 * c + 0.4).sin()) * sway,
                0.06 * (0.41 * c + 2.0).sin() * sway,
            ],
            level: self.activity.clamp(0.0, 1.0),
            lean: self.lean,
            intro: self.intro,
            pulse: self.pulse,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settled(working: bool) -> Frame {
        let mut a = Anim::default();
        let mut f = a.step(0.0, Params { working, open: false, level: None });
        for _ in 0..200 {
            f = a.step(0.016, Params { working, open: false, level: None });
        }
        f
    }

    #[test]
    fn the_orb_is_round_and_transparent_outside() {
        let img = render(64, &settled(false));
        let alpha = |x: usize, y: usize| img[(y * 64 + x) * 4 + 3];
        assert_eq!(alpha(0, 0), 0, "corner");
        assert_eq!(alpha(63, 63), 0, "corner");
        assert!(alpha(32, 32) > 200, "centre is opaque");
    }

    #[test]
    fn premultiplied_colours_never_exceed_alpha() {
        let img = render(48, &settled(true));
        for px in img.chunks(4) {
            assert!(px[0] <= px[3] && px[1] <= px[3] && px[2] <= px[3], "{px:?}");
        }
    }

    #[test]
    fn working_lights_the_orb_up() {
        let sum = |f: &Frame| render(48, f).chunks(4).map(|p| u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2])).sum::<u32>();
        assert!(sum(&settled(true)) > sum(&settled(false)));
    }

    #[test]
    fn a_click_kicks_the_pulse_and_it_settles() {
        let mut a = Anim::default();
        a.step(0.016, Params { working: false, open: false, level: None });
        a.click();
        let kicked = a.step(0.016, Params { working: false, open: false, level: None }).pulse;
        assert!(kicked > 0.0);
        let mut last = kicked;
        for _ in 0..300 {
            last = a.step(0.016, Params { working: false, open: false, level: None }).pulse;
        }
        assert!(last.abs() < 0.01);
    }

    /// Escreve o orbe em `LEGATIA_ORB_OUT` (PPM) e mede o tempo de um quadro: `cargo test orb_preview -- --nocapture`.
    #[test]
    fn orb_preview() {
        let Some(out) = std::env::var_os("LEGATIA_ORB_OUT") else { return };
        let size: usize = std::env::var("LEGATIA_ORB_SIZE").ok().and_then(|s| s.parse().ok()).unwrap_or(256);
        let f = settled(true);
        let t = std::time::Instant::now();
        let img = render(size, &f);
        eprintln!("quadro de {size}px em {:?}", t.elapsed());
        let mut ppm = format!("P6\n{size} {size}\n255\n").into_bytes();
        for px in img.chunks(4) {
            // sobre um fundo escuro, como na dock
            let bg = [24u32, 24, 28];
            let a = u32::from(px[3]);
            for k in 0..3 {
                ppm.push((u32::from(px[k]) + bg[k] * (255 - a) / 255).min(255) as u8);
            }
        }
        std::fs::write(out, ppm).unwrap_or_default();
    }
}
