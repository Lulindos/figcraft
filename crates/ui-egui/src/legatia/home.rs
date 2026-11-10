//! LegatIA: a Home do app nativo. Quem está conectado, os projetos da LegatIA com os arquivos de cada um e as ações
//! de sempre, no padrão da LegatIA: aurora azul ao fundo, saudação em Satoshi Light em duas vozes, vidro líquido com o
//! anel do item ativo do menu, sem linhas entre as seções.
//!
//! Como o botão de salvar (`sync`), isto não fala com o Supabase: pergunta ao programa `legatia-sync`, em 127.0.0.1
//! (`GET /native/overview`), que já tem a sessão da pessoa e a lista da nuvem. A pessoa vê o perfil dela (o da LegatIA,
//! não um do FigCraft), os projetos que o banco deixa ela ver e, em cada um, os arquivos como cartões com a miniatura
//! que vem dentro do próprio `.figcraft` baixado. Um clique abre o arquivo da pasta `LegatIA Design`. O que é da
//! colaboração do FigCraft (aba Online, código de convite, "Primeiros passos") não aparece aqui. Nada disto existe no
//! navegador.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, MutexGuard};

use chrono::{DateTime, FixedOffset, Timelike};
use egui::epaint::PathStroke;
use egui::{Align2, Color32, CornerRadius, Mesh, Pos2, Rect, Sense, Shape, Stroke, Ui, pos2, vec2};
use serde::Deserialize;

use super::host::File;
use crate::FigApp;
use crate::icons::{self, Icon};
use crate::theme::{Tokens, regular, semibold};
use crate::widgets;

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct User {
    pub id: String,
    pub email: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct FileInfo {
    pub id: String,
    pub name: String,
    pub version: i64,
    /// O caminho do arquivo dentro da pasta `LegatIA Design` (`Projeto/Arquivo.figcraft`).
    pub rel: String,
    pub state: String,
    pub updated_at: Option<String>,
    pub updated_by_name: Option<String>,
    pub locked_by_name: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Group {
    pub id: Option<i64>,
    pub name: String,
    pub files: Vec<FileInfo>,
}

/// O que o programa respondeu em `/native/overview`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Overview {
    /// `ok` ou `unpaired` (o programa roda, mas a conta ainda não foi ligada).
    pub state: String,
    pub user: User,
    /// A pasta `LegatIA Design` do computador.
    pub root: String,
    pub groups: Vec<Group>,
}

pub fn parse(body: &str) -> Option<Overview> {
    let overview: Overview = serde_json::from_str(body).ok()?;
    matches!(overview.state.as_str(), "ok" | "unpaired").then_some(overview)
}

struct Shared {
    overview: Option<Overview>,
    /// A primeira consulta ao programa já terminou (até lá a Home diz que está buscando).
    checked: bool,
    /// Miniaturas (PNG em base64) por caminho no disco, com a data do arquivo de onde vieram.
    thumbs: HashMap<String, (u128, Option<String>)>,
    /// A foto de perfil já decodificada, e a hora em que foi buscada (de novo a cada 30 min).
    avatar: Option<egui::ColorImage>,
    avatar_at: Option<std::time::Instant>,
    avatar_texture: Option<egui::TextureHandle>,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    started: bool,
}

static SHARED: LazyLock<Mutex<Shared>> = LazyLock::new(|| Mutex::new(Shared { overview: None, checked: false, thumbs: HashMap::new(), avatar: None, avatar_at: None, avatar_texture: None, started: false }));

fn shared() -> MutexGuard<'static, Shared> {
    SHARED.lock().unwrap_or_else(|e| e.into_inner())
}

// ---------- as palavras: saudação, estado, quem e quando ----------

/// "Bom dia", "Boa tarde" ou "Boa noite", pela hora de Brasília.
pub fn greeting(now: DateTime<FixedOffset>) -> &'static str {
    match now.hour() {
        5..=11 => "Bom dia",
        12..=17 => "Boa tarde",
        _ => "Boa noite",
    }
}

/// A etiqueta sobre a miniatura (vazia: nada a dizer).
pub fn badge(file: &FileInfo) -> String {
    match file.state.as_str() {
        "cloud" => "Baixando…".into(),
        "empty" => "Ainda não salvo".into(),
        "pending" => "Alterado aqui".into(),
        "uploading" => "Enviando…".into(),
        "behind" => "Versão nova na nuvem".into(),
        "conflict" => "Conflito".into(),
        "locked" => format!("Editando: {}", short(file.locked_by_name.as_deref().unwrap_or("outra pessoa"))),
        _ => String::new(),
    }
}

/// A etiqueta pede atenção (alguém editando, conflito, versão nova) em vez de só informar.
pub fn needs_attention(file: &FileInfo) -> bool {
    matches!(file.state.as_str(), "locked" | "conflict" | "behind")
}

fn short(name: &str) -> String {
    let first = name.split_whitespace().next().unwrap_or(name);
    first.chars().take(16).collect()
}

/// "há 5 min", "há 3 h", "ontem 14:05" ou "02/11 10:00": a hora de Brasília, como o resto do app (UTC-3, sem horário de verão).
pub fn when(iso: &str, now: DateTime<FixedOffset>) -> String {
    let Ok(at) = DateTime::parse_from_rfc3339(iso) else { return String::new() };
    let at = at.with_timezone(now.offset());
    let secs = (now - at).num_seconds();
    if secs < 60 {
        return "agora há pouco".into();
    }
    if secs < 3600 {
        return format!("há {} min", secs / 60);
    }
    if secs < 6 * 3600 {
        return format!("há {} h", secs / 3600);
    }
    let clock = format!("{:02}:{:02}", at.hour(), at.minute());
    let days = (now.date_naive() - at.date_naive()).num_days();
    match days {
        0 => format!("hoje {clock}"),
        1 => format!("ontem {clock}"),
        _ => format!("{} {clock}", at.format("%d/%m")),
    }
}

/// A linha sob o nome: "Versão 12 · Luiz · há 3 h".
pub fn detail(file: &FileInfo, now: DateTime<FixedOffset>) -> String {
    if file.state == "empty" {
        return "Sem versões ainda".into();
    }
    let mut parts = Vec::new();
    if file.version > 0 {
        parts.push(format!("Versão {}", file.version));
    }
    if let Some(who) = file.updated_by_name.as_deref().filter(|w| !w.is_empty()) {
        parts.push(short(who));
    }
    if let Some(at) = file.updated_at.as_deref() {
        let text = when(at, now);
        if !text.is_empty() {
            parts.push(text);
        }
    }
    if parts.is_empty() { "Sem versões ainda".into() } else { parts.join(" · ") }
}

fn brasilia_now() -> DateTime<FixedOffset> {
    let offset = FixedOffset::west_opt(3 * 3600).unwrap_or_else(|| FixedOffset::east_opt(0).expect("UTC"));
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    DateTime::from_timestamp(secs, 0).unwrap_or_default().with_timezone(&offset)
}

/// O arquivo no disco: a pasta `LegatIA Design` mais o caminho que o programa deu. `None` se o caminho tenta sair da pasta.
pub fn local_path(root: &str, rel: &str) -> Option<PathBuf> {
    if root.is_empty() || rel.is_empty() || rel.split(['/', '\\']).any(|part| part == ".." || part.is_empty()) || Path::new(rel).is_absolute() {
        return None;
    }
    Some(rel.split('/').fold(PathBuf::from(root), |p, part| p.join(part)))
}

// ---------- a consulta ao programa ----------

#[cfg(not(target_arch = "wasm32"))]
fn ensure_started(ctx: &egui::Context) {
    if cfg!(test) {
        return;
    }
    {
        let mut s = shared();
        if s.started {
            return;
        }
        s.started = true;
    }
    let ctx = ctx.clone();
    let _ = std::thread::Builder::new().name("legatia-home-poll".into()).spawn(move || {
        loop {
            let overview = super::sync::overview_body().and_then(|body| parse(&body));
            let changed = {
                let mut s = shared();
                let changed = s.overview != overview || !s.checked;
                s.overview = overview.clone();
                s.checked = true;
                changed
            };
            if changed {
                ctx.request_repaint();
            }
            if let Some(overview) = overview {
                if load_thumbnails(&overview) {
                    ctx.request_repaint();
                }
                if overview.state == "ok" && load_avatar() {
                    ctx.request_repaint();
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(5));
        }
    });
}

#[cfg(target_arch = "wasm32")]
fn ensure_started(_ctx: &egui::Context) {}

/// Busca a foto de perfil no programa (uma vez, e de novo a cada 30 minutos) e a decodifica. True quando ela mudou.
#[cfg(not(target_arch = "wasm32"))]
fn load_avatar() -> bool {
    {
        let s = shared();
        if s.avatar_at.is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(30 * 60)) {
            return false;
        }
    }
    let image = super::sync::avatar_bytes().and_then(|bytes| figcraft_format::thumbnail::decode_thumbnail(&bytes, 160)).map(|p| egui::ColorImage::from_rgba_unmultiplied([p.width as usize, p.height as usize], &p.rgba));
    let mut s = shared();
    s.avatar_at = Some(std::time::Instant::now());
    let changed = s.avatar.as_ref().map(|i| (i.size, i.pixels.len())) != image.as_ref().map(|i| (i.size, i.pixels.len()));
    if changed {
        s.avatar_texture = None;
    }
    s.avatar = image;
    changed
}

/// A foto de perfil como textura (criada na primeira vez que se desenha), com o recorte quadrado do centro.
fn avatar_texture(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let mut s = shared();
    if s.avatar_texture.is_none() {
        let image = s.avatar.clone()?;
        s.avatar_texture = Some(ctx.load_texture("legatia-avatar", image, egui::TextureOptions::LINEAR));
    }
    s.avatar_texture.clone()
}

/// Tira a miniatura de cada arquivo baixado que ainda não tem uma (ou que mudou). True quando achou alguma nova.
#[cfg(not(target_arch = "wasm32"))]
fn load_thumbnails(overview: &Overview) -> bool {
    use base64::Engine as _;
    let mut found = false;
    for file in overview.groups.iter().flat_map(|g| &g.files) {
        let Some(path) = local_path(&overview.root, &file.rel) else { continue };
        let Ok(stamp) = std::fs::metadata(&path).and_then(|m| m.modified()).map(|t| t.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis())) else {
            continue;
        };
        let key = path.to_string_lossy().into_owned();
        if shared().thumbs.get(&key).is_some_and(|(seen, _)| *seen == stamp) {
            continue;
        }
        // Lê direto do arquivo (um zip): os 26 MB de um design grande não vão para a memória
        let png = std::fs::File::open(&path).ok().and_then(|f| figcraft_format::thumbnail::thumbnail_png(std::io::BufReader::new(f)));
        let encoded = png.map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes));
        found |= encoded.is_some();
        shared().thumbs.insert(key, (stamp, encoded));
    }
    found
}

// ---------- o vidro: aurora, cartão e o anel ----------

/// O raio dos cartões (o das cartas do menu e dos painéis, `PANEL_RADIUS`).
const CARD_RADIUS: f32 = 18.0;

/// A opacidade do anel de vidro, de cima (0) para baixo (1): o gradiente de 180º do item ativo do menu da LegatIA
/// (branco .45, .15, 0, 0, .15, .45).
pub fn ring_alpha(f: f32) -> f32 {
    const STOPS: [(f32, f32); 6] = [(0.0, 0.45), (0.2, 0.15), (0.4, 0.0), (0.6, 0.0), (0.8, 0.15), (1.0, 0.45)];
    let f = f.clamp(0.0, 1.0);
    for pair in STOPS.windows(2) {
        let ((f0, a0), (f1, a1)) = (pair[0], pair[1]);
        if f <= f1 {
            return a0 + (a1 - a0) * ((f - f0) / (f1 - f0));
        }
    }
    STOPS[STOPS.len() - 1].1
}

/// Os pontos do contorno de um retângulo de cantos redondos, no sentido horário a partir do topo. Os lados retos levam
/// um ponto a cada `SIDE_STEP`: a cor do anel varia de ponto a ponto, e com só as pontas o lado ficaria um degradê reto
/// do brilho de cima ao de baixo, sem o vazio do meio.
fn rounded_outline(r: Rect, radius: f32) -> Vec<Pos2> {
    const SIDE_STEP: f32 = 8.0;
    let rad = radius.min(r.width() / 2.0).min(r.height() / 2.0).max(0.0);
    let steps = 10;
    let corners = [
        (pos2(r.max.x - rad, r.min.y + rad), -90.0_f32),
        (pos2(r.max.x - rad, r.max.y - rad), 0.0),
        (pos2(r.min.x + rad, r.max.y - rad), 90.0),
        (pos2(r.min.x + rad, r.min.y + rad), 180.0),
    ];
    let mut points: Vec<Pos2> = Vec::with_capacity(4 * (steps + 1));
    for (center, start) in corners {
        let before = points.last().copied();
        let arc: Vec<Pos2> = (0..=steps)
            .map(|i| {
                let angle = (start + 90.0 * i as f32 / steps as f32).to_radians();
                center + vec2(angle.cos(), angle.sin()) * rad
            })
            .collect();
        // O lado reto entre o fim do canto anterior e o começo deste
        if let (Some(from), Some(to)) = (before, arc.first().copied()) {
            let n = ((to - from).length() / SIDE_STEP).floor() as usize;
            for k in 1..n {
                points.push(from + (to - from) * (k as f32 / n as f32));
            }
        }
        points.extend(arc);
    }
    // O lado do último canto de volta ao primeiro
    if let (Some(from), Some(to)) = (points.last().copied(), points.first().copied()) {
        let n = ((to - from).length() / SIDE_STEP).floor() as usize;
        for k in 1..n {
            points.push(from + (to - from) * (k as f32 / n as f32));
        }
    }
    points
}

/// O anel de vidro do padrão: 1,4 px, branco do alto e do pé que some no meio. `strength` 1 é o do menu.
fn ring(ui: &Ui, rect: Rect, radius: f32, strength: f32) {
    let stroke = PathStroke::new_uv(1.4, move |bounds, p| {
        let f = if bounds.height() > 0.0 { (p.y - bounds.min.y) / bounds.height() } else { 0.0 };
        Color32::from_white_alpha((ring_alpha(f) * strength * 255.0).round().min(255.0) as u8)
    });
    ui.painter().add(Shape::Path(egui::epaint::PathShape::closed_line(rounded_outline(rect.shrink(0.7), radius), stroke)));
}

/// O cartão de vidro: um véu claro com o anel (tema escuro); no claro, o painel com um fio.
fn glass(ui: &Ui, t: &Tokens, rect: Rect, radius: f32, hovered: bool) {
    let p = ui.painter();
    if t.dark {
        p.rect_filled(rect, CornerRadius::same(radius as u8), Color32::from_white_alpha(if hovered { 17 } else { 9 }));
        ring(ui, rect, radius, if hovered { 1.0 } else { 0.75 });
    } else {
        p.rect_filled(rect, CornerRadius::same(radius as u8), if hovered { t.hover } else { t.panel });
        p.rect_stroke(rect, CornerRadius::same(radius as u8), Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    }
}

/// Um brilho redondo que some nas bordas (gradiente radial), pintado como malha.
fn glow(painter: &egui::Painter, center: Pos2, radius: f32, color: Color32) {
    let steps = 56;
    let mut mesh = Mesh::default();
    mesh.colored_vertex(center, color);
    // Dois anéis: o de dentro guarda uma parte da cor, o de fora é transparente (a queda fica suave)
    let inner = Color32::from_rgba_premultiplied(
        (color.r() as f32 * 0.38) as u8,
        (color.g() as f32 * 0.38) as u8,
        (color.b() as f32 * 0.38) as u8,
        (color.a() as f32 * 0.38) as u8,
    );
    for i in 0..steps {
        let a = std::f32::consts::TAU * i as f32 / steps as f32;
        let dir = vec2(a.cos(), a.sin());
        mesh.colored_vertex(center + dir * radius * 0.5, inner);
    }
    for i in 0..steps {
        let a = std::f32::consts::TAU * i as f32 / steps as f32;
        let dir = vec2(a.cos(), a.sin());
        mesh.colored_vertex(center + dir * radius, Color32::TRANSPARENT);
    }
    let n = steps as u32;
    for i in 0..n {
        let next = (i + 1) % n;
        mesh.add_triangle(0, 1 + i, 1 + next);
        mesh.add_triangle(1 + i, 1 + n + i, 1 + n + next);
        mesh.add_triangle(1 + i, 1 + n + next, 1 + next);
    }
    painter.add(Shape::mesh(mesh));
}

/// A aurora azul atrás do conteúdo (tema escuro): índigo no alto à esquerda, azul e um pouco de ciano para a direita.
/// Sempre azul: o laranja é cor de ação e não vai ao fundo. Fica parada (não pede quadros).
pub fn aurora(ui: &Ui, t: &Tokens, area: Rect) {
    if !t.dark {
        return;
    }
    let painter = ui.painter().with_clip_rect(area);
    let w = area.width();
    glow(&painter, pos2(area.min.x + w * 0.16, area.min.y - 30.0), w * 0.62, Color32::from_rgba_unmultiplied(0x0B, 0x3A, 0x8C, 150));
    glow(&painter, pos2(area.min.x + w * 0.64, area.min.y + 10.0), w * 0.46, Color32::from_rgba_unmultiplied(0x31, 0x6A, 0xFD, 62));
    glow(&painter, pos2(area.min.x + w * 0.92, area.min.y + 70.0), w * 0.30, Color32::from_rgba_unmultiplied(0x5A, 0xB4, 0xD2, 30));
}

// ---------- a Home ----------

fn initial(name: &str) -> String {
    name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().to_string()).unwrap_or_else(|| "?".into())
}

fn first_name(name: &str) -> &str {
    name.split_whitespace().next().unwrap_or(name)
}

/// O texto de uma linha só, cortado com reticências quando não cabe.
fn line(ui: &Ui, text: &str, max_w: f32, font: egui::FontId) -> String {
    widgets::elide_font(ui, text, max_w.max(40.0), font)
}

/// A saudação em duas vozes (estilo Apple): a primeira linha na cor do texto, a segunda na do texto suave.
fn hero(ui: &mut Ui, t: &Tokens, overview: Option<&Overview>, checked: bool, now: DateTime<FixedOffset>) {
    let ok = overview.filter(|o| o.state == "ok");
    let (first, second) = match (ok, overview, checked) {
        (Some(o), _, _) => (format!("{}, {}.", greeting(now), first_name(&o.user.name)), "Seus projetos da LegatIA, no seu computador.".to_string()),
        (None, Some(_), _) => ("Bem-vindo ao LegatIA Design.".to_string(), "Ligue o app à sua conta para ver os seus projetos.".to_string()),
        (None, None, false) => ("Bem-vindo ao LegatIA Design.".to_string(), "Buscando os seus projetos…".to_string()),
        (None, None, true) => ("Bem-vindo ao LegatIA Design.".to_string(), "A sincronização com a LegatIA não está rodando neste computador.".to_string()),
    };
    let width = ui.available_width();
    let size = 34.0;
    let (rect, _) = ui.allocate_exact_size(vec2(width, size * 2.5), Sense::hover());
    let pill_w = if ok.is_some() { 240.0 } else { 0.0 };
    let text_w = width - pill_w - 24.0;
    let font = super::fonts::light(size);
    let p = ui.painter();
    p.text(pos2(rect.min.x, rect.min.y + size * 0.62), Align2::LEFT_CENTER, line(ui, &first, text_w, font.clone()), font.clone(), t.text);
    p.text(pos2(rect.min.x, rect.min.y + size * 1.78), Align2::LEFT_CENTER, line(ui, &second, text_w, font.clone()), font, t.text_secondary);
    if let Some(o) = ok {
        // A pílula de vidro de quem está conectado (como a do perfil no site)
        let pill = Rect::from_min_size(pos2(rect.max.x - pill_w, rect.min.y + 4.0), vec2(pill_w, 44.0));
        let resp = ui.interact(pill, ui.id().with("legatia_profile_pill"), Sense::hover());
        glass(ui, t, pill, 22.0, resp.hovered());
        let badge = Rect::from_center_size(pos2(pill.min.x + 24.0, pill.center().y), vec2(30.0, 30.0));
        let p = ui.painter();
        match avatar_texture(ui.ctx()) {
            Some(texture) => {
                // A foto, recortada em quadrado pelo centro e arredondada
                let size = texture.size_vec2();
                let side = size.x.min(size.y).max(1.0);
                let uv = Rect::from_center_size(pos2(0.5, 0.5), vec2(side / size.x.max(1.0), side / size.y.max(1.0)));
                egui::Image::from_texture(egui::load::SizedTexture::new(texture.id(), vec2(30.0, 30.0))).uv(uv).corner_radius(CornerRadius::same(15)).paint_at(ui, badge);
                p.circle_stroke(badge.center(), 15.0, Stroke::new(1.0, Color32::from_white_alpha(40)));
            }
            None => {
                p.circle_filled(badge.center(), 15.0, t.accent);
                p.circle_filled(badge.center() + vec2(0.0, -4.0), 12.0, Color32::from_white_alpha(22));
                p.text(badge.center(), Align2::CENTER_CENTER, initial(&o.user.name), semibold(13.0), t.accent_on);
            }
        }
        let room = pill_w - 52.0 - 34.0;
        p.text(pos2(pill.min.x + 48.0, pill.center().y - 7.0), Align2::LEFT_CENTER, line(ui, &o.user.name, room, semibold(12.5)), semibold(12.5), t.text);
        p.text(pos2(pill.min.x + 48.0, pill.center().y + 9.0), Align2::LEFT_CENTER, line(ui, &o.user.email, room, regular(10.5)), regular(10.5), t.text_secondary);
        // O ponto azul de "conectado", com um halo
        let dot = pos2(pill.max.x - 18.0, pill.center().y);
        p.circle_filled(dot, 7.0, Color32::from_rgba_unmultiplied(0x00, 0x6F, 0xFE, 40));
        p.circle_filled(dot, 3.5, crate::legatia::BLUE_400);
        resp.on_hover_text(format!("{}\nConectado à LegatIA", o.user.email));
    }
}

/// Uma ação: um cartão de vidro com o ícone num bloco azul. A primeira (`primary`) é o azul cheio da marca.
fn action(ui: &mut Ui, t: &Tokens, rect: Rect, icon: Icon, title: &str, sub: &str, primary: bool) -> bool {
    let resp = ui.interact(rect, ui.id().with(("legatia_action", title)), Sense::click());
    crate::a11y::button(&resp, title);
    let hovered = resp.hovered();
    if primary && t.dark {
        let fill = if hovered { crate::legatia::BLUE_400 } else { crate::legatia::BLUE_500 };
        ui.painter().rect_filled(rect, CornerRadius::same(CARD_RADIUS as u8), fill);
        ring(ui, rect, CARD_RADIUS, 1.0);
    } else {
        glass(ui, t, rect, CARD_RADIUS, hovered);
    }
    let on_blue = primary && t.dark;
    let tile = Rect::from_min_size(pos2(rect.min.x + 16.0, rect.center().y - 20.0), vec2(40.0, 40.0));
    let p = ui.painter();
    p.rect_filled(tile, CornerRadius::same(12), if on_blue { Color32::from_white_alpha(30) } else { t.accent });
    icons::paint(p, tile.shrink(11.0), icon, if on_blue { Color32::WHITE } else { t.accent_on });
    let text_w = rect.width() - 80.0 - 12.0;
    let ink = if on_blue { Color32::WHITE } else { t.text };
    let soft = if on_blue { Color32::from_white_alpha(200) } else { t.text_secondary };
    p.text(pos2(rect.min.x + 68.0, rect.center().y - 9.0), Align2::LEFT_CENTER, line(ui, title, text_w, semibold(13.0)), semibold(13.0), ink);
    p.text(pos2(rect.min.x + 68.0, rect.center().y + 10.0), Align2::LEFT_CENTER, line(ui, sub, text_w, regular(11.0)), regular(11.0), soft);
    if hovered {
        ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::PointingHand);
    }
    resp.clicked()
}

fn actions(app: &mut FigApp, ui: &mut Ui, t: &Tokens) {
    let gap = 16.0;
    let width = ui.available_width();
    let columns = if width >= 3.0 * 250.0 + 2.0 * gap {
        3
    } else if width >= 2.0 * 250.0 + gap {
        2
    } else {
        1
    };
    let card_w = ((width - gap * (columns as f32 - 1.0)) / columns as f32).clamp(230.0, 330.0);
    let items: [(Icon, &str, &str); 3] = [
        (Icon::PenNib, "Novo arquivo de design", "Comece com uma tela em branco"),
        (Icon::Board, "Novo quadro", "Notas adesivas, formas e conectores"),
        (Icon::Page, "Abrir arquivo", "Abra .figcraft, .svg ou imagens"),
    ];
    let mut chosen = None;
    for (row, chunk) in items.chunks(columns).enumerate() {
        let (r, _) = ui.allocate_exact_size(vec2(width, 76.0), Sense::hover());
        for (i, (icon, title, sub)) in chunk.iter().enumerate() {
            let rect = Rect::from_min_size(pos2(r.min.x + i as f32 * (card_w + gap), r.min.y), vec2(card_w, 76.0));
            if action(ui, t, rect, *icon, title, sub, row == 0 && i == 0) {
                chosen = Some(row * columns + i);
            }
        }
        ui.add_space(gap);
    }
    match chosen {
        Some(0) => {
            app.run("file.new", serde_json::json!({}));
            app.ui.home = false;
        }
        Some(1) => {
            app.run("file.newBoard", serde_json::json!({}));
            app.ui.home = false;
        }
        Some(2) => app.open_dialog(),
        _ => {}
    }
}

/// Um título de seção em duas vozes: o nome na cor do texto e o resto, mais baixo, na do texto suave.
fn section_title(ui: &mut Ui, t: &Tokens, name: &str, rest: &str, space: f32) {
    ui.add_space(space);
    let font = super::fonts::light(22.0);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
    let g = ui.painter().layout_no_wrap(name.to_string(), font, t.text);
    let w = g.size().x;
    ui.painter().galley(pos2(r.min.x, r.center().y - g.size().y / 2.0), g, t.text);
    if !rest.is_empty() {
        ui.painter().text(pos2(r.min.x + w + 12.0, r.center().y + 2.0), Align2::LEFT_CENTER, rest, regular(12.0), t.text_secondary);
    }
    ui.add_space(8.0);
}

/// A etiqueta sobre a miniatura: uma pílula, azul quando só informa e âmbar quando pede atenção.
fn badge_pill(ui: &Ui, t: &Tokens, area: Rect, text: &str, attention: bool) {
    let p = ui.painter();
    let ink = if attention { Color32::from_rgb(0x1A, 0x12, 0x00) } else { t.accent_on };
    let galley = p.layout_no_wrap(text.to_string(), semibold(10.5), ink);
    let pill = Rect::from_min_size(area.min + vec2(10.0, 10.0), galley.size() + vec2(18.0, 9.0));
    p.rect_filled(pill, CornerRadius::same((pill.height() / 2.0) as u8), if attention { t.warning } else { t.accent });
    p.galley(pill.min + vec2(9.0, 4.5), galley, ink);
}

/// O cartão de um arquivo: vidro com a miniatura num bloco escuro, a etiqueta de estado e o nome com a linha de versão.
fn file_card(ui: &Ui, t: &Tokens, rect: Rect, picture_h: f32, resp: &egui::Response, file: &File, attention: bool) {
    glass(ui, t, rect, CARD_RADIUS, resp.hovered());
    let p = ui.painter();
    let area = Rect::from_min_size(rect.min + vec2(8.0, 8.0), vec2(rect.width() - 16.0, picture_h - 8.0));
    p.rect_filled(area, CornerRadius::same(12), if t.dark { Color32::from_black_alpha(120) } else { t.canvas });
    match super::host::picture(ui, file) {
        Some(texture) => {
            let size = texture.size_vec2();
            let inner = area.shrink(area.height() * 0.07);
            let fit = (inner.width() / size.x.max(1.0)).min(inner.height() / size.y.max(1.0));
            let image = Rect::from_center_size(inner.center(), size * fit);
            p.image(texture.id(), image, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        None => icons::paint(p, Rect::from_center_size(area.center(), vec2(28.0, 28.0)), Icon::Page, t.icon_secondary),
    }
    if !file.badge.is_empty() {
        badge_pill(ui, t, area, &file.badge, attention);
    }
    let footer_top = area.max.y + 8.0;
    let (x, w) = (rect.min.x + 18.0, rect.width() - 36.0);
    let name = widgets::elide_font(ui, &file.name, w, semibold(12.5));
    p.text(pos2(x, footer_top + 14.0), Align2::LEFT_CENTER, name, semibold(12.5), t.text);
    let detail = widgets::elide_font(ui, &file.detail, w, regular(11.0));
    p.text(pos2(x, footer_top + 32.0), Align2::LEFT_CENTER, detail, regular(11.0), t.text_secondary);
    if resp.has_focus() {
        p.rect_stroke(rect, CornerRadius::same(CARD_RADIUS as u8), Stroke::new(2.0, t.focus), egui::StrokeKind::Outside);
    }
}

/// Quantos cartões cabem numa linha de `width` e a largura de cada um: de 280 a 380 px, para caber mais projetos por tela.
pub fn grid_shape(width: f32, gap: f32) -> (usize, f32) {
    const MIN: f32 = 280.0;
    const MAX: f32 = 380.0;
    let columns = (((width + gap) / (MIN + gap)).floor() as usize).max(1);
    let card_w = ((width - gap * (columns as f32 - 1.0)) / columns as f32).clamp(MIN.min(width.max(120.0)), MAX);
    (columns, card_w)
}

/// Os cartões de um projeto, quantos couberem na linha; devolve o arquivo escolhido.
fn file_grid(ui: &mut Ui, t: &Tokens, items: &[(File, bool)]) -> Option<String> {
    let mut chosen = None;
    let gap = 18.0;
    let (columns, card_w) = grid_shape(ui.available_width(), gap);
    let picture_h = (card_w * 0.6).round();
    let card_h = picture_h + 66.0;
    for row in items.chunks(columns) {
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), card_h), Sense::hover());
        for (i, (file, attention)) in row.iter().enumerate() {
            let rect = Rect::from_min_size(pos2(r.min.x + i as f32 * (card_w + gap), r.min.y), vec2(card_w, card_h));
            let resp = ui.interact(rect, egui::Id::new(("legatia_home_file", &file.id)), Sense::click());
            crate::a11y::button(&resp, &file.name);
            if ui.is_rect_visible(rect) {
                file_card(ui, t, rect, picture_h, &resp, file, *attention);
            }
            if resp.hovered() {
                ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::PointingHand);
            }
            if resp.clicked() {
                chosen = Some(file.id.clone());
            }
        }
        ui.add_space(gap);
    }
    chosen
}

/// Os projetos da LegatIA com os arquivos de cada um. Um clique abre o arquivo da pasta.
fn projects(app: &mut FigApp, ui: &mut Ui, t: &Tokens, overview: &Overview, now: DateTime<FixedOffset>) {
    let with_files: Vec<&Group> = overview.groups.iter().filter(|g| !g.files.is_empty()).collect();
    let total: usize = with_files.iter().map(|g| g.files.len()).sum();
    let summary = match total {
        0 => "nenhum arquivo ainda".to_string(),
        1 => "1 arquivo".to_string(),
        n => format!("{n} arquivos em {}", if with_files.len() == 1 { "1 projeto".to_string() } else { format!("{} projetos", with_files.len()) }),
    };
    section_title(ui, t, "Projetos", &summary, 28.0);
    if total == 0 {
        ui.label(egui::RichText::new("Os arquivos de design dos seus projetos aparecem aqui. Crie um arquivo, salve na pasta LegatIA Design e ele vai para a nuvem.").font(regular(12.0)).color(t.text_secondary));
    }
    let mut opened: Option<(PathBuf, bool)> = None;
    for group in &with_files {
        ui.add_space(14.0);
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
        let label = ui.painter().layout_no_wrap(group.name.clone(), semibold(13.0), t.text);
        let w = label.size().x;
        ui.painter().galley(pos2(r.min.x, r.center().y - label.size().y / 2.0), label, t.text);
        // A contagem numa pílula de vidro
        let count = ui.painter().layout_no_wrap(group.files.len().to_string(), semibold(10.5), t.text_secondary);
        let chip = Rect::from_min_size(pos2(r.min.x + w + 10.0, r.center().y - 9.0), vec2(count.size().x + 14.0, 18.0));
        ui.painter().rect_filled(chip, CornerRadius::same(9), if t.dark { Color32::from_white_alpha(14) } else { t.hover });
        ui.painter().galley(pos2(chip.min.x + 7.0, chip.center().y - count.size().y / 2.0), count, t.text_secondary);
        ui.add_space(10.0);
        let items: Vec<(File, bool)> = {
            let s = shared();
            group
                .files
                .iter()
                .map(|f| {
                    let path = local_path(&overview.root, &f.rel);
                    let thumbnail = path.as_ref().and_then(|p| s.thumbs.get(p.to_string_lossy().as_ref())).and_then(|(_, png)| png.clone()).unwrap_or_default();
                    (File { id: f.id.clone(), name: f.name.clone(), detail: detail(f, now), badge: badge(f), thumbnail }, needs_attention(f))
                })
                .collect()
        };
        if let Some(id) = file_grid(ui, t, &items) {
            if let Some(file) = group.files.iter().find(|f| f.id == id) {
                if let Some(path) = local_path(&overview.root, &file.rel) {
                    opened = Some((path, file.state == "empty"));
                }
            }
        }
    }
    let without: Vec<&str> = overview.groups.iter().filter(|g| g.files.is_empty()).map(|g| g.name.as_str()).collect();
    if !without.is_empty() {
        let rest = if without.len() == 1 { "1 ainda sem arquivo de design".to_string() } else { format!("{} ainda sem arquivo de design", without.len()) };
        section_title(ui, t, "Outros projetos", &rest, 32.0);
        project_chips(ui, t, &without);
    }
    if let Some((path, never_saved)) = opened {
        open_local(app, &path, never_saved);
    }
}

/// Os projetos que ainda não têm arquivo de design, como pílulas de vidro que quebram em quantas linhas precisar.
fn project_chips(ui: &mut Ui, t: &Tokens, names: &[&str]) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
        for name in names {
            let shown = widgets::elide_font(ui, name.trim(), 240.0, regular(11.5));
            let galley = ui.painter().layout_no_wrap(shown.clone(), regular(11.5), t.text_secondary);
            let (r, resp) = ui.allocate_exact_size(vec2(galley.size().x + 28.0, 28.0), Sense::hover());
            glass(ui, t, r, 14.0, resp.hovered());
            ui.painter().galley(pos2(r.min.x + 14.0, r.center().y - galley.size().y / 2.0), galley, t.text_secondary);
            if shown != name.trim() {
                resp.on_hover_text(name.trim());
            }
        }
    });
}

/// Abre o arquivo da pasta, ou diz por que ainda não dá (nunca salvo, ou ainda baixando).
fn open_local(app: &mut FigApp, path: &Path, never_saved: bool) {
    if path.is_file() {
        if app.run_user("file.open", serde_json::json!({"path": path.to_string_lossy()})).is_some() {
            app.ui.home = false;
        }
    } else if never_saved {
        app.toast("Este arquivo ainda não tem nenhuma versão salva na LegatIA.");
    } else {
        app.toast("O arquivo ainda está sendo baixado para o computador. Tente de novo em alguns segundos.");
    }
}

/// O arquivo de nome `name`, no projeto `group`, aparece numa busca por `query` (já em minúsculas)? Procura nos dois.
pub fn matches(group: &str, name: &str, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    !query.is_empty() && (name.to_lowercase().contains(&query) || group.to_lowercase().contains(&query))
}

/// A busca da Home (o campo da barra lateral): os arquivos da LegatIA cujo nome ou projeto tem o texto, como cartões. Devolve quantos.
pub fn search(app: &mut FigApp, ui: &mut Ui, t: &Tokens, query: &str) -> usize {
    let Some(overview) = shared().overview.clone().filter(|o| o.state == "ok") else { return 0 };
    let now = brasilia_now();
    let found: Vec<(&Group, &FileInfo)> = overview.groups.iter().flat_map(|g| g.files.iter().map(move |f| (g, f))).filter(|(g, f)| matches(&g.name, &f.name, query)).collect();
    if found.is_empty() {
        return 0;
    }
    let rest = if found.len() == 1 { "1 resultado".to_string() } else { format!("{} resultados", found.len()) };
    section_title(ui, t, "Na LegatIA", &rest, 0.0);
    let items: Vec<(File, bool)> = {
        let s = shared();
        found
            .iter()
            .map(|(g, f)| {
                let path = local_path(&overview.root, &f.rel);
                let thumbnail = path.as_ref().and_then(|p| s.thumbs.get(p.to_string_lossy().as_ref())).and_then(|(_, png)| png.clone()).unwrap_or_default();
                (File { id: f.id.clone(), name: f.name.clone(), detail: format!("{} · {}", g.name, detail(f, now)), badge: badge(f), thumbnail }, needs_attention(f))
            })
            .collect()
    };
    if let Some(id) = file_grid(ui, t, &items) {
        if let Some((_, file)) = found.iter().find(|(_, f)| f.id == id) {
            if let Some(path) = local_path(&overview.root, &file.rel) {
                open_local(app, &path, file.state == "empty");
            }
        }
    }
    found.len()
}

/// O convite para ligar o app à conta (o programa roda, mas a conta ainda não foi ligada).
fn connect_card(ui: &mut Ui, t: &Tokens) {
    ui.add_space(8.0);
    let width = ui.available_width().min(640.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, 96.0), Sense::hover());
    glass(ui, t, rect, CARD_RADIUS, false);
    let p = ui.painter();
    p.text(pos2(rect.min.x + 24.0, rect.min.y + 32.0), Align2::LEFT_CENTER, "Falta ligar o app à sua conta", semibold(14.0), t.text);
    p.text(pos2(rect.min.x + 24.0, rect.min.y + 56.0), Align2::LEFT_CENTER, "No site, em Design, clique em “App no computador”.", regular(11.5), t.text_secondary);
    let button = Rect::from_min_size(pos2(rect.max.x - 24.0 - 168.0, rect.center().y - 18.0), vec2(168.0, 36.0));
    let resp = ui.interact(button, ui.id().with("legatia_connect"), Sense::click());
    ui.painter().rect_filled(button, CornerRadius::same(18), if resp.hovered() { crate::legatia::BLUE_400 } else { crate::legatia::BLUE_500 });
    ui.painter().text(button.center(), Align2::CENTER_CENTER, "Conectar à LegatIA", semibold(12.0), Color32::WHITE);
    crate::a11y::button(&resp, "Conectar à LegatIA");
    if resp.clicked() {
        ui.ctx().open_url(egui::OpenUrl::new_tab(super::sync::SITE));
    }
}

/// Os arquivos abertos faz pouco neste computador (os recentes do FigCraft), com o desenho dele, abaixo dos projetos.
fn recents(app: &mut FigApp, ui: &mut Ui, t: &Tokens) {
    if crate::home_recents::listed(&app.session.prefs.recent_files).next().is_none() {
        return;
    }
    section_title(ui, t, "Neste computador", "arquivos que você abriu faz pouco", 36.0);
    crate::home_recents::section(app, ui, t, "");
}

/// A Home do app do computador. Sempre desenha (devolve true): a do FigCraft só serve ao navegador.
pub fn layout(app: &mut FigApp, ui: &mut Ui, t: &Tokens) -> bool {
    ensure_started(ui.ctx());
    let (overview, checked) = {
        let s = shared();
        (s.overview.clone(), s.checked)
    };
    let now = brasilia_now();
    ui.add_space(12.0);
    hero(ui, t, overview.as_ref(), checked, now);
    ui.add_space(12.0);
    actions(app, ui, t);
    match overview.as_ref() {
        Some(o) if o.state == "ok" => projects(app, ui, t, o, now),
        Some(_) => connect_card(ui, t),
        None => {}
    }
    recents(app, ui, t);
    ui.add_space(40.0);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(s).unwrap()
    }

    fn file(state: &str) -> FileInfo {
        FileInfo { id: "a".into(), name: "Home".into(), version: 12, rel: "Loja/Home.figcraft".into(), state: state.into(), ..Default::default() }
    }

    #[test]
    fn parses_the_programs_overview() {
        let body = r#"{"state":"ok","user":{"id":"u","email":"maria@legat.ia","name":"Maria Silva"},"root":"C:/x",
            "groups":[{"id":1,"name":"Loja","files":[{"id":"a","name":"Home","version":3,"rel":"Loja/Home.figcraft","state":"synced","updatedAt":"2026-11-10T18:00:00+00:00","updatedByName":"Luiz","lockedByName":null}]},
                      {"id":null,"name":"Geral","files":[]}]}"#;
        let overview = parse(body).unwrap();
        assert_eq!((overview.user.name.as_str(), overview.groups.len()), ("Maria Silva", 2));
        assert_eq!(overview.groups[0].files[0].updated_by_name.as_deref(), Some("Luiz"));
        assert_eq!(overview.groups[1].id, None);
        assert!(parse(r#"{"state":"unpaired"}"#).is_some());
        assert!(parse(r#"{"state":"outro"}"#).is_none());
        assert!(parse("não é json").is_none());
    }

    #[test]
    fn the_greeting_follows_the_hour_in_brasilia() {
        assert_eq!(greeting(at("2026-11-10T06:00:00-03:00")), "Bom dia");
        assert_eq!(greeting(at("2026-11-10T11:59:00-03:00")), "Bom dia");
        assert_eq!(greeting(at("2026-11-10T12:00:00-03:00")), "Boa tarde");
        assert_eq!(greeting(at("2026-11-10T17:59:00-03:00")), "Boa tarde");
        assert_eq!(greeting(at("2026-11-10T18:00:00-03:00")), "Boa noite");
        assert_eq!(greeting(at("2026-11-10T03:00:00-03:00")), "Boa noite");
    }

    #[test]
    fn time_is_said_in_brasilia_time() {
        let now = at("2026-11-10T15:00:00-03:00");
        assert_eq!(when("2026-11-10T17:59:40+00:00", now), "agora há pouco");
        assert_eq!(when("2026-11-10T17:45:00+00:00", now), "há 15 min");
        assert_eq!(when("2026-11-10T15:00:00+00:00", now), "há 3 h");
        // 5h30 antes ainda conta em horas
        assert_eq!(when("2026-11-10T12:30:00+00:00", now), "há 5 h");
        assert_eq!(when("2026-11-10T08:00:00+00:00", now), "hoje 05:00");
        assert_eq!(when("2026-11-09T20:05:00+00:00", now), "ontem 17:05");
        assert_eq!(when("2026-11-02T13:00:00+00:00", now), "02/11 10:00");
        assert_eq!(when("lixo", now), "");
    }

    #[test]
    fn the_line_under_the_name_gathers_what_is_known() {
        let now = at("2026-11-10T15:00:00-03:00");
        let mut f = file("synced");
        f.updated_by_name = Some("Luiz Fernando".into());
        f.updated_at = Some("2026-11-10T17:45:00+00:00".into());
        assert_eq!(detail(&f, now), "Versão 12 · Luiz · há 15 min");
        assert_eq!(detail(&FileInfo { state: "empty".into(), ..Default::default() }, now), "Sem versões ainda");
        // Um arquivo vazio na nuvem não mostra "agora há pouco" só porque acabou de ser criado
        let mut empty = file("empty");
        empty.version = 0;
        empty.updated_at = Some("2026-11-10T17:59:50+00:00".into());
        assert_eq!(detail(&empty, now), "Sem versões ainda");
    }

    #[test]
    fn the_tag_says_what_needs_attention_and_stays_quiet_when_synced() {
        assert_eq!(badge(&file("synced")), "");
        assert_eq!(badge(&file("pending")), "Alterado aqui");
        assert_eq!(badge(&file("cloud")), "Baixando…");
        assert_eq!(badge(&file("behind")), "Versão nova na nuvem");
        let mut locked = file("locked");
        locked.locked_by_name = Some("Maria Fernanda".into());
        assert_eq!(badge(&locked), "Editando: Maria");
        assert!(needs_attention(&locked) && needs_attention(&file("conflict")) && needs_attention(&file("behind")));
        assert!(!needs_attention(&file("synced")) && !needs_attention(&file("pending")) && !needs_attention(&file("cloud")));
    }

    #[test]
    fn a_path_never_leaves_the_legatia_folder() {
        let ok = local_path("C:/Users/x/LegatIA Design", "Loja do Zé/Home.figcraft").unwrap();
        assert!(ok.ends_with("Home.figcraft") && ok.to_string_lossy().contains("Loja do Zé"));
        assert!(local_path("C:/x", "../fora.figcraft").is_none());
        assert!(local_path("C:/x", "Loja/../../fora.figcraft").is_none());
        assert!(local_path("C:/x", "").is_none());
        assert!(local_path("", "Loja/Home.figcraft").is_none());
        assert!(local_path("C:/x", "/etc/passwd").is_none());
    }

    #[test]
    fn the_profile_shows_a_letter() {
        assert_eq!(initial("maria silva"), "M");
        assert_eq!(initial("  Édson"), "É");
        assert_eq!(initial(""), "?");
        assert_eq!(first_name("Maria Silva"), "Maria");
    }

    #[test]
    fn the_grid_fits_more_cards_on_a_wide_window_and_one_on_a_narrow_one() {
        // 1400 px de largura útil: quatro colunas, cada uma entre 280 e 380
        let (columns, w) = grid_shape(1400.0, 18.0);
        assert_eq!(columns, 4);
        assert!((280.0..=380.0).contains(&w));
        assert!(w * 4.0 + 18.0 * 3.0 <= 1400.0 + 0.5);
        // Uma janela estreita ainda mostra um cartão inteiro, nunca zero colunas
        let (columns, w) = grid_shape(300.0, 18.0);
        assert_eq!(columns, 1);
        assert!(w <= 300.0);
        assert_eq!(grid_shape(100.0, 18.0).0, 1);
        // Largura enorme: cartões não passam de 380
        assert!(grid_shape(5000.0, 18.0).1 <= 380.0);
    }

    #[test]
    fn the_search_looks_in_the_file_name_and_in_the_project() {
        assert!(matches("Scalefy 2.0", "Dashboard", "dash"));
        assert!(matches("Scalefy 2.0", "Dashboard", "SCALEFY"));
        assert!(matches("Geral", "LegatIA Design System", " design "));
        assert!(!matches("Scalefy 2.0", "Dashboard", "confido"));
        assert!(!matches("Scalefy 2.0", "Dashboard", "   "));
    }

    #[test]
    fn the_glass_ring_is_bright_at_the_ends_and_gone_in_the_middle() {
        // O gradiente do item ativo do menu: branco .45 nas pontas, .15 a 20% e 80%, nada no meio
        assert!((ring_alpha(0.0) - 0.45).abs() < 1e-6 && (ring_alpha(1.0) - 0.45).abs() < 1e-6);
        assert!((ring_alpha(0.2) - 0.15).abs() < 1e-6 && (ring_alpha(0.8) - 0.15).abs() < 1e-6);
        assert!(ring_alpha(0.5).abs() < 1e-6);
        assert!(ring_alpha(0.1) > ring_alpha(0.2) && ring_alpha(0.3) < ring_alpha(0.2));
        // Fora do intervalo vale o da ponta
        assert_eq!(ring_alpha(-1.0), ring_alpha(0.0));
        assert_eq!(ring_alpha(2.0), ring_alpha(1.0));
    }

    #[test]
    fn the_outline_of_a_card_stays_inside_it_and_closes_the_corners() {
        let r = Rect::from_min_size(pos2(10.0, 20.0), vec2(200.0, 100.0));
        let points = rounded_outline(r, 18.0);
        // Os quatro cantos (11 pontos cada) e pontos pelos lados retos, no máximo 8 px entre dois vizinhos
        assert!(points.len() > 44);
        assert!(points.iter().all(|p| r.expand(0.01).contains(*p)));
        assert!(points.iter().zip(points.iter().cycle().skip(1)).all(|(a, b)| (*b - *a).length() <= 8.5), "um lado ficou sem pontos no meio");
        // O primeiro ponto está no topo, depois do canto de cima à direita
        assert!((points[0].y - r.min.y).abs() < 0.01);
        // O raio nunca passa de metade do lado menor
        let pill = rounded_outline(Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 30.0)), 99.0);
        assert!(pill.iter().all(|p| (0.0..=100.0).contains(&p.x) && (0.0..=30.0).contains(&p.y)));
    }
}
