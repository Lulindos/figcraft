//! LegatIA: a Home do app nativo. Quem está conectado, os projetos da LegatIA com os arquivos de cada um e as ações
//! de sempre, no padrão da LegatIA: a luz do topo do site (aurora e ASCII) ao fundo, saudação em Satoshi Light em duas vozes, vidro líquido com o
//! anel do item ativo do menu, sem linhas entre as seções.
//!
//! Como o botão de salvar (`sync`), isto não fala com o Supabase: pergunta ao programa `legatia-sync`, em 127.0.0.1
//! (`GET /native/overview`), que já tem a sessão da pessoa e a lista da nuvem. A pessoa vê o perfil dela (o da LegatIA,
//! não um do FigCraft), os projetos que o banco deixa ela ver e, em cada um, os arquivos como cartões com a miniatura
//! que vem dentro do próprio `.figcraft` baixado. Um clique abre o arquivo da pasta `LegatIA Design`. O que é da
//! colaboração do FigCraft (aba Online, código de convite, "Primeiros passos") não aparece aqui. Nada disto existe no
//! navegador.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, MutexGuard};

use chrono::{DateTime, FixedOffset, Timelike};
use egui::epaint::PathStroke;
use egui::{Align2, Color32, CornerRadius, Pos2, Rect, Sense, Shape, Stroke, Ui, pos2, vec2};
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
    /// Quem travou o arquivo (para mostrar a foto dessa pessoa).
    pub locked_by_id: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Group {
    pub id: Option<i64>,
    pub name: String,
    /// A subpasta de `LegatIA Design` que representa o projeto (vazia: o programa não disse).
    pub folder: String,
    /// O projeto tem capa (`projects.cover_image`); a imagem vem de `/native/cover?project=<id>`.
    pub has_cover: bool,
    pub files: Vec<FileInfo>,
}

/// O que o programa respondeu em `/native/overview`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Overview {
    /// Há conexão com a nuvem agora? Sem ela a Home mostra o que o programa já tinha e o que se salva espera para subir.
    #[serde(default = "yes")]
    pub online: bool,
    /// `ok` ou `unpaired` (o programa roda, mas a conta ainda não foi ligada).
    pub state: String,
    pub user: User,
    /// A pasta `LegatIA Design` do computador.
    pub root: String,
    pub groups: Vec<Group>,
}

fn yes() -> bool {
    true
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
    /// Onde, neste computador, ficam os arquivos que estão na nuvem (em minúsculas, com `/`): a Home não os repete em "Neste computador".
    cloud_paths: HashSet<String>,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    started: bool,
}

static SHARED: LazyLock<Mutex<Shared>> = LazyLock::new(|| Mutex::new(Shared { overview: None, checked: false, thumbs: HashMap::new(), avatar: None, avatar_at: None, avatar_texture: None, cloud_paths: HashSet::new(), started: false }));

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
    badge_for(file, true)
}

/// A etiqueta com a conexão em conta: sem ela o que espera para subir "aguarda conexão" e o que não foi baixado "só está na nuvem".
pub fn badge_for(file: &FileInfo, online: bool) -> String {
    if !online {
        match file.state.as_str() {
            "pending" | "uploading" => return "Aguardando conexão".into(),
            "cloud" => return "Só na nuvem".into(),
            _ => {}
        }
    }
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

/// Um caminho do disco para comparar: barras para frente e minúsculas (o Windows não distingue).
fn norm(path: &str) -> String {
    path.replace('\\', "/").to_lowercase()
}

/// Os arquivos da nuvem no disco, normalizados.
fn cloud_set(overview: &Overview) -> HashSet<String> {
    overview.groups.iter().flat_map(|g| &g.files).filter_map(|f| local_path(&overview.root, &f.rel)).map(|p| norm(&p.to_string_lossy())).collect()
}

/// Este arquivo está na nuvem da LegatIA (a Home o mostra em "Na nuvem", não em "Neste computador")?
pub fn is_cloud_file(path: &str) -> bool {
    !cfg!(test) && shared().cloud_paths.contains(&norm(path))
}

/// A foto de perfil de quem está conectado, como textura, para o editor (a barra de pessoas). `None` sem foto ou sem o programa.
pub fn my_avatar(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    ensure_started(ctx);
    avatar_texture(ctx)
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
                s.cloud_paths = overview.as_ref().map(cloud_set).unwrap_or_default();
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

/// O que a Home sabe agora (a última resposta do programa de sincronização), ou `None`.
pub fn current() -> Option<Overview> {
    shared().overview.clone()
}

/// O raio dos cartões (o das cartas do menu e dos painéis, `PANEL_RADIUS`).
/// Os cartões são os mesmos de "Neste computador" (os do FigCraft): cantos de 8 px, fundo do painel e um fio. Só o perfil é vidro.
const CARD_RADIUS: f32 = 8.0;

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
pub(super) fn ring(ui: &Ui, rect: Rect, radius: f32, strength: f32) {
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

/// O cartão liso: o painel com um fio (mais claro ao passar o mouse), igual ao dos arquivos recentes do FigCraft.
fn plain(ui: &Ui, t: &Tokens, rect: Rect, hovered: bool) {
    let p = ui.painter();
    let radius = CornerRadius::same(CARD_RADIUS as u8);
    p.rect_filled(rect, radius, if hovered { t.hover } else { t.panel });
    p.rect_stroke(rect, radius, Stroke::new(1.0, if hovered { t.border_strong } else { t.border }), egui::StrokeKind::Inside);
}

/// A luz do topo da Home: a aurora do site com o campo ASCII por cima e o degradê para o fundo (`aurora.rs`). Sempre azul.
pub fn aurora(ui: &Ui, t: &Tokens, area: Rect, reduce_motion: bool) {
    super::aurora::paint(ui, t, area, reduce_motion, brasilia_now());
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
        (Some(o), _, _) => (
            format!("{}, {}.", greeting(now), first_name(&o.user.name)),
            if o.online { "Seus projetos da LegatIA, no seu computador.".to_string() } else { "Sem conexão: você vê o que está salvo neste PC.".to_string() },
        ),
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
        if o.online {
            p.circle_filled(dot, 7.0, Color32::from_rgba_unmultiplied(0x00, 0x6F, 0xFE, 40));
            p.circle_filled(dot, 3.5, crate::legatia::BLUE_400);
            resp.on_hover_text(format!("{}\nConectado à LegatIA", o.user.email));
        } else {
            p.circle_filled(dot, 3.5, t.text_secondary);
            resp.on_hover_text(format!("{}\nSem conexão com a LegatIA. O que você salvar aqui vai para a nuvem quando a conexão voltar.", o.user.email));
        }
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
        plain(ui, t, rect, hovered);
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
    section_heading(ui, t, name, rest, space, false);
}

/// Como [`section_title`]; `cloud` põe a nuvenzinha azul antes do nome (o que está no Supabase da LegatIA).
fn section_heading(ui: &mut Ui, t: &Tokens, name: &str, rest: &str, space: f32, cloud: bool) {
    ui.add_space(space);
    let font = super::fonts::light(22.0);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
    let left = if cloud { 34.0 } else { 0.0 };
    if cloud {
        cloud_icon(ui.painter(), pos2(r.min.x + 13.0, r.center().y), 24.0, crate::legatia::BLUE_400);
    }
    let g = ui.painter().layout_no_wrap(name.to_string(), font, t.text);
    let w = g.size().x;
    ui.painter().galley(pos2(r.min.x + left, r.center().y - g.size().y / 2.0), g, t.text);
    if !rest.is_empty() {
        ui.painter().text(pos2(r.min.x + left + w + 12.0, r.center().y + 2.0), Align2::LEFT_CENTER, rest, regular(12.0), t.text_secondary);
    }
    ui.add_space(8.0);
}

/// Uma nuvenzinha cheia de `width` pontos de largura, centrada em `c` (cor sólida: as partes se sobrepõem).
fn cloud_icon(p: &egui::Painter, c: Pos2, width: f32, color: Color32) {
    let u = width / 13.2;
    let c = c + vec2(-0.2 * u, 0.9 * u);
    p.circle_filled(c + vec2(-3.6 * u, 0.8 * u), 2.8 * u, color);
    p.circle_filled(c + vec2(-0.4 * u, -1.4 * u), 4.0 * u, color);
    p.circle_filled(c + vec2(4.0 * u, 0.8 * u), 2.8 * u, color);
    p.rect_filled(Rect::from_min_max(c + vec2(-3.6 * u, 0.4 * u), c + vec2(4.0 * u, 3.6 * u)), CornerRadius::ZERO, color);
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

/// Quem está editando o arquivo: a foto dessa pessoa (ou a inicial) no canto do cartão.
struct Locker {
    texture: Option<egui::TextureHandle>,
    initial: String,
}

/// Tudo o que um cartão de arquivo mostra.
struct Card {
    file: File,
    attention: bool,
    /// O arquivo está na nuvem da LegatIA: a nuvenzinha no canto.
    cloud: bool,
    /// A capa do projeto, no lugar da miniatura (arquivo ainda sem desenho baixado).
    cover: Option<egui::TextureHandle>,
    locker: Option<Locker>,
}

/// Monta o cartão de um arquivo da nuvem. `with_project`: a linha de baixo diz também o projeto (na busca).
fn make_card(ctx: &egui::Context, overview: &Overview, group: &Group, f: &FileInfo, now: DateTime<FixedOffset>, with_project: bool, cover: Option<&egui::TextureHandle>) -> Card {
    let path = local_path(&overview.root, &f.rel);
    let thumbnail = {
        let s = shared();
        path.as_ref().and_then(|p| s.thumbs.get(p.to_string_lossy().as_ref())).and_then(|(_, png)| png.clone()).unwrap_or_default()
    };
    let has_picture = !thumbnail.is_empty();
    let line = if with_project { format!("{} · {}", group.name, detail(f, now)) } else { detail(f, now) };
    let locker = (f.state == "locked").then(|| Locker {
        texture: f.locked_by_id.as_deref().and_then(|id| super::images::avatar(ctx, id)),
        initial: initial(f.locked_by_name.as_deref().unwrap_or("?")),
    });
    Card {
        file: File { id: f.id.clone(), name: f.name.clone(), detail: line, badge: badge_for(f, overview.online), thumbnail },
        attention: needs_attention(f),
        cloud: true,
        cover: if has_picture { None } else { cover.cloned() },
        locker,
    }
}

/// Uma imagem que preenche `rect` (recorte pelo centro, sem esticar), com cantos arredondados e escurecida por `tint`.
fn paint_cover(ui: &Ui, rect: Rect, texture: &egui::TextureHandle, radius: u8, tint: Color32) {
    let uv = super::images::cover_uv(texture.size_vec2(), rect.width() / rect.height().max(1.0));
    egui::Image::from_texture(egui::load::SizedTexture::new(texture.id(), rect.size())).uv(uv).corner_radius(CornerRadius::same(radius)).tint(tint).paint_at(ui, rect);
}

/// O cartão de um arquivo: vidro com a miniatura (ou a capa do projeto) num bloco escuro, a etiqueta de estado, a nuvenzinha,
/// a foto de quem está editando e o nome com a linha de versão.
fn file_card(ui: &Ui, t: &Tokens, rect: Rect, picture_h: f32, resp: &egui::Response, card: &Card) {
    let file = &card.file;
    plain(ui, t, rect, resp.hovered());
    let p = ui.painter();
    let area = Rect::from_min_size(rect.min + vec2(8.0, 8.0), vec2(rect.width() - 16.0, picture_h - 8.0));
    p.rect_filled(area, CornerRadius::same(6), if t.dark { Color32::from_black_alpha(120) } else { t.canvas });
    match (super::host::picture(ui, file), card.cover.as_ref()) {
        (Some(texture), _) => {
            let size = texture.size_vec2();
            let inner = area.shrink(area.height() * 0.07);
            let fit = (inner.width() / size.x.max(1.0)).min(inner.height() / size.y.max(1.0));
            let image = Rect::from_center_size(inner.center(), size * fit);
            p.image(texture.id(), image, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        (None, Some(cover)) => paint_cover(ui, area, cover, 6, Color32::from_gray(150)),
        (None, None) => icons::paint(p, Rect::from_center_size(area.center(), vec2(28.0, 28.0)), Icon::Page, t.icon_secondary),
    }
    if !file.badge.is_empty() {
        badge_pill(ui, t, area, &file.badge, card.attention);
    }
    if let Some(locker) = &card.locker {
        // Quem está editando: a foto num círculo com o anel âmbar de "atenção"
        let c = pos2(area.max.x - 20.0, area.min.y + 20.0);
        p.circle_filled(c, 15.0, t.warning);
        match &locker.texture {
            Some(texture) => {
                let uv = super::images::cover_uv(texture.size_vec2(), 1.0);
                egui::Image::from_texture(egui::load::SizedTexture::new(texture.id(), vec2(26.0, 26.0))).uv(uv).corner_radius(CornerRadius::same(13)).paint_at(ui, Rect::from_center_size(c, vec2(26.0, 26.0)));
            }
            None => {
                p.circle_filled(c, 13.0, Color32::from_rgb(0x1A, 0x12, 0x00));
                p.text(c, Align2::CENTER_CENTER, &locker.initial, semibold(12.0), t.warning);
            }
        }
    }
    if card.cloud {
        let pill = Rect::from_min_size(pos2(area.max.x - 34.0, area.max.y - 26.0), vec2(26.0, 18.0));
        p.rect_filled(pill, CornerRadius::same(9), Color32::from_black_alpha(150));
        cloud_icon(p, pill.center(), 13.0, Color32::WHITE);
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
fn file_grid(ui: &mut Ui, t: &Tokens, items: &[Card]) -> Option<String> {
    let mut chosen = None;
    let gap = 18.0;
    let (columns, card_w) = grid_shape(ui.available_width(), gap);
    let picture_h = (card_w * 0.6).round();
    let card_h = picture_h + 66.0;
    for row in items.chunks(columns) {
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), card_h), Sense::hover());
        for (i, card) in row.iter().enumerate() {
            let rect = Rect::from_min_size(pos2(r.min.x + i as f32 * (card_w + gap), r.min.y), vec2(card_w, card_h));
            let resp = ui.interact(rect, egui::Id::new(("legatia_home_file", &card.file.id)), Sense::click());
            crate::a11y::button(&resp, &card.file.name);
            if ui.is_rect_visible(rect) {
                file_card(ui, t, rect, picture_h, &resp, card);
            }
            if resp.hovered() {
                ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::PointingHand);
            }
            if resp.clicked() {
                chosen = Some(card.file.id.clone());
            }
        }
        ui.add_space(gap);
    }
    chosen
}

/// O cabeçalho de um projeto: uma linha compacta com a capa dele (a do Supabase) num quadradinho, o nome e quantos arquivos tem.
/// Devolve a capa, que os cartões ainda sem desenho usam no lugar da miniatura.
fn banner(ui: &mut Ui, t: &Tokens, group: &Group) -> Option<egui::TextureHandle> {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), Sense::hover());
    let cover = if group.has_cover { group.id.and_then(|id| super::images::cover(ui.ctx(), id)) } else { None };
    if !ui.is_rect_visible(rect) {
        return cover;
    }
    let tile = Rect::from_min_size(pos2(rect.min.x, rect.center().y - 22.0), vec2(44.0, 44.0));
    let p = ui.painter();
    match &cover {
        Some(texture) => {
            paint_cover(ui, tile, texture, 10, Color32::WHITE);
            ui.painter().rect_stroke(tile, CornerRadius::same(10), Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
        }
        None => {
            p.rect_filled(tile, CornerRadius::same(10), t.accent);
            cloud_icon(p, tile.center(), 22.0, t.accent_on);
        }
    }
    let p = ui.painter();
    if cover.is_some() {
        // A nuvenzinha no canto da capa
        let pill = Rect::from_min_size(pos2(tile.max.x - 20.0, tile.max.y - 14.0), vec2(24.0, 18.0));
        p.rect_filled(pill, CornerRadius::same(9), t.accent);
        cloud_icon(p, pill.center(), 13.0, t.accent_on);
    }
    let x = tile.max.x + 18.0;
    let room = rect.max.x - x - 8.0;
    let count = match group.files.len() {
        1 => "1 arquivo".to_string(),
        n => format!("{n} arquivos"),
    };
    p.text(pos2(x, rect.center().y - 9.0), Align2::LEFT_CENTER, line(ui, group.name.trim(), room, semibold(16.0)), semibold(16.0), t.text);
    ui.painter().text(pos2(x, rect.center().y + 11.0), Align2::LEFT_CENTER, format!("{count} · projeto na nuvem"), regular(11.5), t.text_secondary);
    cover
}

/// Os projetos da LegatIA ("Na nuvem") com os arquivos de cada um. Um clique abre o arquivo da pasta.
fn projects(app: &mut FigApp, ui: &mut Ui, t: &Tokens, overview: &Overview, now: DateTime<FixedOffset>) {
    let with_files: Vec<&Group> = overview.groups.iter().filter(|g| !g.files.is_empty()).collect();
    let total: usize = with_files.iter().map(|g| g.files.len()).sum();
    let summary = match total {
        0 => "nenhum arquivo ainda".to_string(),
        1 => "1 arquivo".to_string(),
        n => format!("{n} arquivos em {}", if with_files.len() == 1 { "1 projeto".to_string() } else { format!("{} projetos", with_files.len()) }),
    };
    section_heading(ui, t, "Na nuvem", &summary, 28.0, true);
    if total == 0 {
        ui.label(egui::RichText::new("Os arquivos de design dos seus projetos aparecem aqui. Crie um arquivo, salve na pasta LegatIA Design e ele vai para a nuvem.").font(regular(12.0)).color(t.text_secondary));
    }
    let mut opened: Option<(PathBuf, bool)> = None;
    for group in &with_files {
        ui.add_space(14.0);
        let cover = banner(ui, t, group);
        ui.add_space(12.0);
        let cards: Vec<Card> = group.files.iter().map(|f| make_card(ui.ctx(), overview, group, f, now, false, cover.as_ref())).collect();
        if let Some(id) = file_grid(ui, t, &cards) {
            if let Some(file) = group.files.iter().find(|f| f.id == id) {
                if let Some(path) = local_path(&overview.root, &file.rel) {
                    opened = Some((path, file.state == "empty"));
                }
            }
        }
    }
    let without: Vec<&Group> = overview.groups.iter().filter(|g| g.files.is_empty()).collect();
    if !without.is_empty() {
        let rest = if without.len() == 1 { "1 ainda sem arquivo de design".to_string() } else { format!("{} ainda sem arquivo de design", without.len()) };
        section_title(ui, t, "Outros projetos", &rest, 32.0);
        if let Some(group) = project_search(ui, t, &without) {
            // Um arquivo novo para esse projeto: o "Salvar na LegatIA" dele já abre com o projeto marcado
            let folder = if group.folder.is_empty() { super::save_dialog::sanitize_name(&group.name, "Projeto") } else { group.folder.clone() };
            super::save_dialog::prefer_project(&folder);
            app.run("file.new", serde_json::json!({}));
            app.ui.home = false;
            app.toast(format!("Arquivo novo para “{}”. Clique em Salvar na LegatIA quando quiser guardar.", group.name.trim()));
        }
    }
    if let Some((path, never_saved)) = opened {
        open_local(app, &path, never_saved);
    }
}

/// Quantos projetos a lista de resultados mostra de uma vez.
const RESULTS: usize = 8;

/// Os projetos que casam com o texto (nome contém o texto, sem diferença de maiúsculas), na ordem em que vieram.
pub fn project_matches<'a>(groups: &[&'a Group], query: &str) -> Vec<&'a Group> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    groups.iter().copied().filter(|g| g.name.to_lowercase().contains(&needle)).collect()
}

/// A busca dos projetos que ainda não têm arquivo de design: um campo e, ao digitar, até oito resultados.
/// Devolve o projeto em que a pessoa clicou (ou o primeiro, com Enter) para criar um arquivo nele.
fn project_search<'a>(ui: &mut Ui, t: &Tokens, groups: &[&'a Group]) -> Option<&'a Group> {
    let id = egui::Id::new("legatia_new_in_project");
    let mut query: String = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    let width = ui.available_width().min(520.0);
    let hint = format!("Buscar entre os {} projetos sem arquivo…", groups.len());
    let mut enter = false;
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), widgets::FIELD_H), Sense::hover());
    ui.scope_builder(egui::UiBuilder::new().max_rect(Rect::from_min_size(row.min, vec2(width, widgets::FIELD_H))), |ui| {
        let response = widgets::text_input(ui, id.with("field"), &mut query, width, &hint);
        crate::a11y::text(&response, &hint, &query);
        enter = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    });
    ui.data_mut(|d| d.insert_temp(id, query.clone()));
    ui.add_space(8.0);
    let found = project_matches(groups, &query);
    if found.is_empty() {
        if !query.trim().is_empty() {
            ui.label(egui::RichText::new("Nenhum projeto com esse nome.").font(regular(11.5)).color(t.text_secondary));
        }
        return None;
    }
    let mut chosen = enter.then(|| found[0]);
    for group in found.iter().take(RESULTS) {
        let (r, resp) = ui.allocate_exact_size(vec2(width, 36.0), Sense::click());
        plain(ui, t, r, resp.hovered());
        let name = line(ui, group.name.trim(), width - 190.0, regular(12.5));
        let p = ui.painter();
        p.text(pos2(r.min.x + 16.0, r.center().y), Align2::LEFT_CENTER, name, regular(12.5), t.text);
        p.text(pos2(r.max.x - 16.0, r.center().y), Align2::RIGHT_CENTER, "Novo arquivo neste projeto", regular(11.0), if resp.hovered() { t.accent_text } else { t.text_secondary });
        crate::a11y::button(&resp, group.name.trim());
        if resp.hovered() {
            ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::PointingHand);
        }
        if resp.clicked() {
            chosen = Some(*group);
        }
        ui.add_space(6.0);
    }
    if found.len() > RESULTS {
        ui.label(egui::RichText::new(format!("e mais {}: continue digitando para achar o projeto.", found.len() - RESULTS)).font(regular(11.0)).color(t.text_secondary));
    }
    if chosen.is_some() {
        ui.data_mut(|d| d.insert_temp(id, String::new()));
    }
    chosen
}

/// Abre o arquivo da pasta, ou diz por que ainda não dá (nunca salvo, ou ainda baixando).
fn open_local(app: &mut FigApp, path: &Path, never_saved: bool) {
    let offline = current().is_some_and(|o| !o.online);
    if path.is_file() {
        if app.run_user("file.open", serde_json::json!({"path": path.to_string_lossy()})).is_some() {
            app.ui.home = false;
        }
    } else if never_saved {
        app.toast("Este arquivo ainda não tem nenhuma versão salva na LegatIA.");
    } else if offline {
        app.toast("Sem conexão: este arquivo ainda não foi baixado para o computador.");
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
    let items: Vec<Card> = found.iter().map(|(g, f)| make_card(ui.ctx(), &overview, g, f, now, true, None)).collect();
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
    plain(ui, t, rect, false);
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
    section_title(ui, t, "Neste computador", "arquivos abertos faz pouco, fora da nuvem", 36.0);
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
    fn the_overview_carries_the_cover_and_who_locked() {
        let body = r#"{"state":"ok","root":"C:/x","groups":[{"id":3,"name":"Scalefy","hasCover":true,"files":[{"id":"a","name":"Home","state":"locked","lockedByName":"Ana","lockedById":"22222222-2222-4222-8222-222222222222"}]},{"id":4,"name":"Sem capa","files":[]}]}"#;
        let o = parse(body).unwrap();
        assert!(o.groups[0].has_cover && !o.groups[1].has_cover);
        assert_eq!(o.groups[0].files[0].locked_by_id.as_deref(), Some("22222222-2222-4222-8222-222222222222"));
    }

    #[test]
    fn the_cloud_files_are_known_by_a_normalized_path() {
        let o = Overview {
            state: "ok".into(),
            root: r"C:\Users\Ana\LegatIA Design".into(),
            groups: vec![Group { name: "Loja".into(), files: vec![FileInfo { rel: "Loja/Home.figcraft".into(), ..Default::default() }], ..Default::default() }],
            ..Default::default()
        };
        let set = cloud_set(&o);
        // O app guarda o caminho do jeito do Windows e com outra caixa: ainda é o mesmo arquivo
        assert!(set.contains(&norm(r"c:\users\ana\legatia design\Loja\Home.figcraft")));
        assert!(!set.contains(&norm(r"C:\Users\Ana\Documentos\Home.figcraft")));
    }

    #[test]
    fn the_project_search_matches_part_of_the_name_and_shows_nothing_for_an_empty_query() {
        let g = |name: &str| Group { name: name.into(), ..Default::default() };
        let (a, b, c) = (g("Scalefy 2.0"), g("Confido"), g("Loja do Zé"));
        let all = [&a, &b, &c];
        assert_eq!(project_matches(&all, "SCALE").iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["Scalefy 2.0"]);
        assert_eq!(project_matches(&all, " o ").len(), 2, "Confido e Loja do Zé têm \"o\"; Scalefy 2.0 não");
        assert!(project_matches(&all, "   ").is_empty());
        assert!(project_matches(&all, "xyz").is_empty());
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
    fn without_a_connection_the_tags_wait_for_it() {
        assert_eq!(badge_for(&file("pending"), false), "Aguardando conexão");
        assert_eq!(badge_for(&file("uploading"), false), "Aguardando conexão");
        assert_eq!(badge_for(&file("cloud"), false), "Só na nuvem");
        assert_eq!(badge_for(&file("synced"), false), "");
        // Com conexão, as de sempre
        assert_eq!(badge_for(&file("pending"), true), "Alterado aqui");
        assert_eq!(badge_for(&file("cloud"), true), "Baixando…");
    }

    #[test]
    fn a_program_that_does_not_say_is_online() {
        assert!(parse(r#"{"state":"ok"}"#).unwrap().online);
        assert!(!parse(r#"{"state":"ok","online":false}"#).unwrap().online);
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
