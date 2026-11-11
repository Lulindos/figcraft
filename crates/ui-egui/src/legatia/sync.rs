//! LegatIA: como está o arquivo aberto em relação à LegatIA, e o botão que salva e envia.
//!
//! O app nativo não fala com o Supabase: o programa `legatia-sync` (tools/legatia-sync, no repositório do site)
//! espelha os arquivos da conta numa pasta do computador. Este módulo só pergunta a ele, em 127.0.0.1, como está o
//! arquivo aberto, com o segredo que ele guarda em `%APPDATA%\LegatIA Design\native.token`, e mostra o estado no
//! botão primário, no lugar do "Compartilhar" (como o "Salvar na LegatIA" do site): o estado, e um clique que salva no
//! disco e manda enviar na hora. Sem o programa rodando, o botão continua o do FigCraft. Nada disto existe no navegador
//! (lá a página hospedeira salva).

use std::sync::{Mutex, MutexGuard};

use crate::FigApp;

/// A página do site onde a pessoa liga o app à conta ("App no computador").
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const SITE: &str = "https://app.legatia.com.br/dashboard/design";

/// O que o programa respondeu sobre o arquivo aberto.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Reply {
    pub state: String,
    pub detail: String,
    /// O programa alcança a nuvem agora? Sem conexão o app salva neste PC e o programa envia quando ela voltar.
    pub online: bool,
    /// Enviando: quantos bytes já foram e o total (a barra do botão).
    pub sent: Option<u64>,
    pub total: Option<u64>,
    /// Enviado há pouco por este PC: a versão que a nuvem aceitou e o tamanho (a confirmação).
    pub sent_version: Option<i64>,
    pub sent_bytes: Option<u64>,
    /// `too_large`: o tamanho do arquivo que a nuvem recusou.
    pub size_bytes: Option<u64>,
}

/// Bytes em MB para as mensagens ("170 MB", "4,2 MB").
pub fn mb(bytes: u64) -> String {
    let v = bytes as f64 / 1_048_576.0;
    if v >= 10.0 { format!("{} MB", v.round() as i64) } else { format!("{:.1} MB", v).replace('.', ",") }
}

/// O aviso que o app mostra quando o estado do arquivo `before` vira `after` (o mesmo arquivo): a confirmação de um envio que
/// a nuvem aceitou, ou o motivo de a nuvem recusar por tamanho. `None`: nada a dizer.
pub fn notice(before: Option<&Reply>, after: &Reply) -> Option<String> {
    let was = |s: &str| before.is_some_and(|b| b.state == s);
    if let (Some(version), Some(bytes)) = (after.sent_version, after.sent_bytes) {
        if before.is_none_or(|b| b.sent_version != Some(version)) && (was("uploading") || was("pending") || was("new")) {
            return Some(format!("Enviado e confirmado na LegatIA: versão {version} · {}.", mb(bytes)));
        }
    }
    if after.state == "too_large" && !was("too_large") {
        let size = after.size_bytes.map_or_else(|| after.detail.clone(), mb);
        return Some(format!("Este arquivo tem {size} e a nuvem não aceitou por tamanho. Ele continua salvo neste PC. Quando o limite subir, clique em “Grande demais · tentar” para enviar de novo."));
    }
    None
}

/// O botão primário (no lugar do "Compartilhar", como o "Salvar na LegatIA" do site): o que ele diz e o que um clique faz.
#[derive(Clone, Debug, PartialEq)]
pub struct Primary {
    /// `save` (salva e envia), `connect` (abre o site) ou vazio (só informa).
    pub id: &'static str,
    pub label: String,
    pub tip: String,
    pub busy: bool,
    pub disabled: bool,
    /// Enviando: de 0 a 1 (a barra dentro do botão).
    pub progress: Option<f32>,
}

struct Shared {
    /// O caminho do arquivo aberto agora, que o fio de consulta manda ao programa.
    path: Option<String>,
    /// `None`: o programa não está rodando (ou o segredo não existe), então o botão do app fica como o do FigCraft.
    reply: Option<Reply>,
    /// O arquivo a que `reply` se refere (a resposta pode ser de um arquivo que já não está na frente).
    reply_for: Option<String>,
    /// Avisos para o app mostrar (confirmação de envio, recusa por tamanho), tirados no próximo quadro.
    notices: Vec<String>,
    /// A consulta ao programa só existe no desktop.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    started: bool,
}

static SHARED: Mutex<Shared> = Mutex::new(Shared { path: None, reply: None, reply_for: None, notices: Vec::new(), started: false });

/// O estado (`synced`, `unpaired`, `outside`…) do arquivo `path`, se a última resposta do programa é dele.
pub fn state_for(path: &str) -> Option<String> {
    let s = shared();
    (s.reply_for.as_deref() == Some(path)).then(|| s.reply.as_ref().map(|r| r.state.clone())).flatten()
}

fn shared() -> MutexGuard<'static, Shared> {
    SHARED.lock().unwrap_or_else(|e| e.into_inner())
}

/// O botão para este arquivo: `dirty` é "tem alteração ainda não salva no disco".
pub fn primary(reply: &Reply, dirty: bool, has_path: bool) -> Primary {
    let p = |id: &'static str, label: &str, tip: &str, busy: bool, disabled: bool| Primary { id, label: label.to_string(), tip: tip.to_string(), busy, disabled, progress: None };
    if reply.state == "unpaired" {
        return p("connect", "Conectar à LegatIA", "Abre a página Design do site: clique em “App no computador” para ligar este app à sua conta.", false, false);
    }
    // Sem conexão: salvar é salvar neste PC, e o que já foi salvo espera a conexão (o programa envia sozinho quando ela voltar)
    if !reply.online {
        let waiting = "Salvo neste PC. Vai para a LegatIA assim que a conexão voltar.";
        if !has_path || reply.state == "outside" {
            return p("save", "Salvar neste PC", "Sem conexão: o arquivo fica salvo na pasta da LegatIA neste PC e vai para a nuvem quando a conexão voltar.", false, false);
        }
        if dirty {
            return p("save", "Salvar neste PC", "Sem conexão: salva aqui e envia para a LegatIA quando a conexão voltar.", false, false);
        }
        if matches!(reply.state.as_str(), "pending" | "new" | "uploading") {
            return p("", "Aguardando rede", waiting, false, true);
        }
    }
    if !has_path {
        return p("save", "Salvar na LegatIA", "Salve o arquivo dentro da pasta LegatIA Design para ele ir para a nuvem.", false, false);
    }
    if reply.state == "outside" {
        return p("save", "Salvar na LegatIA", "Este arquivo está fora da pasta LegatIA Design. Clique para escolher o projeto e salvar uma cópia lá.", false, false);
    }
    if matches!(reply.state.as_str(), "missing" | "none") {
        return p("", "Só neste PC", "Este arquivo está fora da pasta LegatIA Design e não vai para a nuvem.", false, true);
    }
    if dirty {
        return p("save", "Salvar na LegatIA", "Salva e envia para a nuvem agora.", false, false);
    }
    match reply.state.as_str() {
        "uploading" => match (reply.sent, reply.total) {
            (Some(sent), Some(total)) if total > 0 => {
                let k = (sent as f64 / total as f64).clamp(0.0, 1.0);
                Primary {
                    id: "",
                    label: format!("Enviando {}%", (k * 100.0).floor() as i64),
                    tip: format!("Enviando para a nuvem: {} de {}.", mb(sent), mb(total)),
                    busy: true,
                    disabled: false,
                    progress: Some(k as f32),
                }
            }
            _ => p("", "Enviando…", "Enviando a versão salva para a nuvem.", true, false),
        },
        "too_large" => p(
            "save",
            "Grande demais · tentar",
            &format!("Este arquivo tem {} e a nuvem recusou por tamanho (o limite do armazenamento). Ele continua salvo neste PC. Clique para tentar de novo (depois que o limite subir).", reply.size_bytes.map_or_else(|| reply.detail.clone(), mb)),
            false,
            false,
        ),
        "pending" => p("save", "Enviar à LegatIA", "Salvo no disco e ainda não enviado. Clique para enviar agora.", false, false),
        "new" => p("save", "Novo · enviar", "Arquivo novo na pasta: clique para criar na nuvem agora.", false, false),
        "locked" => Primary {
            id: "",
            label: format!("Editando: {}", short(&reply.detail)),
            tip: "Outra pessoa está editando. O envio acontece quando a trava soltar (até 15 min).".into(),
            busy: false,
            disabled: true,
            progress: None,
        },
        "conflict" => p("", "Conflito", "A nuvem mudou junto: a sua cópia está na pasta _conflitos e este arquivo é o da nuvem.", false, true),
        "behind" => p("", "Nuvem mais nova", "Feche e abra o arquivo para ver a versão da nuvem.", false, true),
        // Acabou de enviar daqui: a confirmação fica no botão (a nuvem aceitou esta versão, com este tamanho)
        "synced" if reply.sent_version.is_some() => Primary {
            id: "",
            label: format!("Enviado ✓ versão {}", reply.sent_version.unwrap_or_default()),
            tip: format!("A nuvem confirmou: versão {} · {}.", reply.sent_version.unwrap_or_default(), reply.sent_bytes.map_or_else(String::new, mb)),
            busy: false,
            disabled: true,
            progress: Some(1.0),
        },
        "synced" => Primary { id: "", label: "Salvo na LegatIA".into(), tip: format!("Igual à nuvem · {}.", reply.detail), busy: false, disabled: true, progress: None },
        _ => p("", "LegatIA", "Estado desconhecido.", false, true),
    }
}

/// O nome de quem está editando, curto para o botão não invadir o resto da barra.
fn short(name: &str) -> String {
    let mut out: String = name.chars().take(12).collect();
    if name.chars().count() > 12 {
        out.push('…');
    }
    out
}

fn active_path(app: &FigApp) -> Option<String> {
    // Na aba ao vivo, o arquivo que importa é o da pasta de onde ela nasceu (é ele que vai para a nuvem)
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(origin) = super::live_edit::live_origin(app) {
        return Some(origin);
    }
    app.session.docs.get(app.session.active).and_then(|d| d.path.clone())
}

fn current_primary(app: &FigApp) -> Option<Primary> {
    let reply = shared().reply.clone()?;
    let ds = app.session.docs.get(app.session.active)?;
    #[cfg(not(target_arch = "wasm32"))]
    if super::live_edit::live_origin(app).is_some() {
        return Some(live_primary(&reply, ds.is_dirty(), super::live_edit::other_saver()));
    }
    Some(primary(&reply, ds.is_dirty(), ds.path.is_some()))
}

/// O botão na aba ao vivo: diz que está ao vivo e como está a gravação. `saver`: quem grava, quando não sou eu.
pub fn live_primary(reply: &Reply, dirty: bool, saver: Option<String>) -> Primary {
    let tip_auto = "Todos neste arquivo veem as mudanças na hora. A sessão grava na LegatIA sozinha, alguns segundos depois que a mexida para.";
    let p = |id: &'static str, label: &str, tip: String, busy: bool| Primary { id, label: label.to_string(), tip, busy, disabled: id.is_empty(), progress: None };
    if let Some(name) = saver {
        return p("", "Ao vivo", format!("Todos neste arquivo veem as mudanças na hora. Quem grava na LegatIA é {name} (entrou primeiro), sozinho."), false);
    }
    if !reply.online {
        return p("", "Ao vivo · sem rede", "Sem conexão com a LegatIA: a gravação espera a rede voltar.".to_string(), false);
    }
    match (dirty, reply.state.as_str()) {
        (true, _) => p("save", "Ao vivo · salvar", format!("{tip_auto} Clique para gravar agora."), false),
        (false, "uploading" | "pending") => p("", "Ao vivo · enviando…", tip_auto.to_string(), true),
        _ => p("", "Ao vivo · salvo", tip_auto.to_string(), false),
    }
}

/// Liga a consulta ao programa e diz qual é o arquivo aberto. O estado agora aparece no botão primário, não na barra de abas.
pub fn reserved(app: &FigApp, ctx: &egui::Context) -> f32 {
    ensure_started(ctx);
    shared().path = active_path(app);
    0.0
}

/// A barra de abas não tem mais o botão do estado (ver [`host_frame`]): devolve o retângulo que recebeu.
pub fn chip(_app: &mut FigApp, _ui: &mut egui::Ui, _t: &crate::theme::Tokens, left_of: egui::Rect) -> egui::Rect {
    left_of
}

/// O que o app diz ao `host` a cada quadro: o botão primário (estado do arquivo, no lugar do Compartilhar) e o espaço do orbe
/// no fim da barra de ferramentas; e o que acontece com os cliques desde o quadro anterior.
#[cfg(not(target_arch = "wasm32"))]
pub fn host_frame(app: &mut FigApp, ctx: &egui::Context) {
    // O quadro "Salvar na LegatIA" (projeto e nome), quando aberto
    super::save_dialog::show(app, ctx);
    // Editar ao vivo: o pedido do menu das pessoas, o que o programa respondeu e o aviso de que a sessão segue aberta
    super::live_edit::frame(app, ctx);
    // Um arquivo que a pessoa pediu na Home e que o programa foi buscar na nuvem: abre quando chega
    super::home::fetch_frame(app);
    // Avisos do envio: confirmação de que a nuvem aceitou, ou o motivo de recusar por tamanho
    for msg in std::mem::take(&mut shared().notices) {
        app.toast(msg);
    }
    // Demonstrar: a câmera que acompanha o enxame e a seleção que segue o quadro da vez
    #[cfg(not(target_arch = "wasm32"))]
    super::reveal::frame(app, ctx);
    for event in super::host::take_events() {
        if event.get("type").and_then(|t| t.as_str()) != Some("button") {
            continue;
        }
        match event.get("id").and_then(|i| i.as_str()) {
            Some("save") => save_now(app, ctx),
            Some("connect") => ctx.open_url(egui::OpenUrl::new_tab(SITE)),
            _ => {}
        }
    }
    let mut host = serde_json::json!({ "dockSlot": 36.0 });
    if let Some(p) = current_primary(app) {
        host["primary"] = serde_json::json!({ "id": p.id, "label": p.label, "tip": p.tip, "busy": p.busy, "disabled": p.disabled, "progress": p.progress });
    }
    static LAST: Mutex<String> = Mutex::new(String::new());
    let text = host.to_string();
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    if *last != text {
        let _ = super::host::set(host);
        *last = text;
    }
}

/// Salva no disco (como Ctrl+S) e manda o programa enviar este arquivo agora, sem esperar a próxima verificação.
pub fn save_now(app: &mut FigApp, ctx: &egui::Context) {
    // A aba de uma sessão ao vivo grava o desenho no arquivo da pasta de onde ela nasceu (e o programa o envia como qualquer versão)
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(origin) = super::live_edit::origin_for_save(app) {
        super::live_edit::save_to_origin(app, ctx, origin);
        return;
    }
    // Um arquivo fora da pasta da LegatIA vai para ela pelo quadro (projeto e nome), não salvo no lugar onde está
    #[cfg(not(target_arch = "wasm32"))]
    if shared().reply.as_ref().is_some_and(|r| r.state == "outside") && super::save_dialog::open(app) {
        return;
    }
    app.save();
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(path) = active_path(app) {
        if offline() {
            // Sem conexão não há o que enviar agora: fica salvo aqui e o programa envia quando ela voltar
            app.toast("Sem conexão: salvo neste PC. Vai para a LegatIA quando a conexão voltar.");
        } else {
            send_now(path, ctx);
        }
    }
    #[cfg(target_arch = "wasm32")]
    let _ = ctx;
}

/// Um pedido POST ao programa (`/native/live...`): o código HTTP e o corpo, qualquer que seja o código. `None`: o programa não respondeu.
#[cfg(not(target_arch = "wasm32"))]
pub fn post(target: &str, secs: u64) -> Option<(u16, String)> {
    let token = native::token()?;
    native::http_full("POST", target, &token, secs)
}

/// Um texto pronto para ir numa consulta (`?path=...`).
#[cfg(not(target_arch = "wasm32"))]
pub fn encode_query(s: &str) -> String {
    native::encode(s)
}

/// O programa respondeu que está sem conexão com a nuvem (e a conta já estava ligada).
pub fn offline() -> bool {
    shared().reply.as_ref().is_some_and(|r| !r.online && r.state != "unpaired")
}

/// Manda o programa enviar este arquivo agora, sem esperar a próxima verificação.
#[cfg(not(target_arch = "wasm32"))]
pub fn send_now(path: String, ctx: &egui::Context) {
    let ctx = ctx.clone();
    let _ = std::thread::Builder::new().name("legatia-sync-now".into()).spawn(move || {
        if let Some(token) = native::token() {
            let _ = native::http("POST", &format!("/native/sync-now?path={}", native::encode(&path)), &token, 180);
        }
        refresh(Some(&path), &ctx);
    });
}

/// A foto de perfil de quem está conectado (os bytes da imagem), ou `None` sem o programa ou sem foto.
#[cfg(not(target_arch = "wasm32"))]
pub fn avatar_bytes() -> Option<Vec<u8>> {
    image_bytes("/native/avatar")
}

/// Uma imagem do programa (`/native/avatar?user=<id>`, `/native/cover?project=<id>`), ou `None` sem o programa, sem rede ou sem imagem.
#[cfg(not(target_arch = "wasm32"))]
pub fn image_bytes(target: &str) -> Option<Vec<u8>> {
    let token = native::token()?;
    native::http_bytes("GET", target, &token, 20)
}

/// A Home do app: o corpo de `/native/overview` (quem está conectado e os projetos com os arquivos), ou `None` sem o programa.
#[cfg(not(target_arch = "wasm32"))]
pub fn overview_body() -> Option<String> {
    let token = native::token()?;
    native::http("GET", "/native/overview", &token, 15)
}

#[cfg(not(target_arch = "wasm32"))]
fn refresh(path: Option<&str>, ctx: &egui::Context) {
    let reply = native::poll(path);
    let changed = {
        let mut s = shared();
        // Um envio que terminou (ou foi recusado) vira aviso; abrir um arquivo que a nuvem recusou por tamanho também avisa
        if let Some(after) = reply.as_ref() {
            let before = if s.reply_for.as_deref() == path { s.reply.as_ref() } else { None };
            if let Some(msg) = notice(before, after) {
                s.notices.push(msg);
            }
        }
        let changed = s.reply != reply || s.reply_for.as_deref() != path;
        s.reply = reply;
        s.reply_for = path.map(str::to_string);
        changed
    };
    if changed {
        ctx.request_repaint();
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn ensure_started(ctx: &egui::Context) {
    // Nos testes não há programa, nem fio: o botão fica de fora.
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
    let _ = std::thread::Builder::new().name("legatia-sync-poll".into()).spawn(move || {
        loop {
            let path = shared().path.clone();
            refresh(path.as_deref(), &ctx);
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    });
}

#[cfg(target_arch = "wasm32")]
fn ensure_started(_ctx: &egui::Context) {}

/// A conversa com o programa: HTTP/1.1 simples em 127.0.0.1, só com a biblioteca padrão.
#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpStream};
    use std::path::PathBuf;
    use std::time::Duration;

    use super::Reply;

    fn port() -> u16 {
        std::env::var("LEGATIA_SYNC_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(47821)
    }

    /// O segredo que o programa guarda ao lado da sessão; sem ele (programa nunca rodou) não há o que consultar.
    pub fn token() -> Option<String> {
        let dir = std::env::var_os("LEGATIA_SYNC_STATE")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("LegatIA Design")))
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config").join("LegatIA Design")))?;
        let text = std::fs::read_to_string(dir.join("native.token")).ok()?;
        let token = text.trim().to_string();
        (!token.is_empty()).then_some(token)
    }

    pub fn encode(s: &str) -> String {
        let mut out = String::with_capacity(s.len() * 3);
        for b in s.bytes() {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                out.push(b as char);
            } else {
                out.push_str(&format!("%{b:02X}"));
            }
        }
        out
    }

    /// O corpo da resposta 200, ou `None` (programa fora do ar, segredo errado, erro).
    pub fn http(method: &str, target: &str, token: &str, read_secs: u64) -> Option<String> {
        let port = port();
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(400)).ok()?;
        stream.set_read_timeout(Some(Duration::from_secs(read_secs))).ok()?;
        stream.set_write_timeout(Some(Duration::from_secs(3))).ok()?;
        let request = format!("{method} {target} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Legatia-Native: {token}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        stream.write_all(request.as_bytes()).ok()?;
        let mut raw = String::new();
        stream.read_to_string(&mut raw).ok()?;
        let (head, body) = raw.split_once("\r\n\r\n")?;
        head.starts_with("HTTP/1.1 200").then(|| body.to_string())
    }

    /// Como [`http`], mas devolve também o código quando não é 200 (um erro do programa traz o motivo no corpo).
    pub fn http_full(method: &str, target: &str, token: &str, read_secs: u64) -> Option<(u16, String)> {
        let port = port();
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(400)).ok()?;
        stream.set_read_timeout(Some(Duration::from_secs(read_secs))).ok()?;
        stream.set_write_timeout(Some(Duration::from_secs(3))).ok()?;
        let request = format!("{method} {target} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Legatia-Native: {token}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        stream.write_all(request.as_bytes()).ok()?;
        let mut raw = String::new();
        stream.read_to_string(&mut raw).ok()?;
        let (head, body) = raw.split_once("\r\n\r\n")?;
        let code = head.split_whitespace().nth(1)?.parse().ok()?;
        Some((code, body.to_string()))
    }

    /// Como [`http`], mas devolve o corpo como bytes (uma imagem não é texto).
    pub fn http_bytes(method: &str, target: &str, token: &str, read_secs: u64) -> Option<Vec<u8>> {
        let port = port();
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(400)).ok()?;
        stream.set_read_timeout(Some(Duration::from_secs(read_secs))).ok()?;
        stream.set_write_timeout(Some(Duration::from_secs(3))).ok()?;
        let request = format!("{method} {target} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Legatia-Native: {token}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        stream.write_all(request.as_bytes()).ok()?;
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).ok()?;
        let split = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
        let head = std::str::from_utf8(&raw[..split]).ok()?;
        head.starts_with("HTTP/1.1 200").then(|| raw[split + 4..].to_vec())
    }

    pub fn parse(body: &str) -> Option<Reply> {
        let v: serde_json::Value = serde_json::from_str(body).ok()?;
        Some(Reply {
            state: v.get("state")?.as_str()?.to_string(),
            detail: v.get("detail").and_then(|d| d.as_str()).unwrap_or("").to_string(),
            // Um programa antigo não diz: vale como com conexão
            online: v.get("online").and_then(|o| o.as_bool()).unwrap_or(true),
            sent: v.get("sent").and_then(serde_json::Value::as_u64),
            total: v.get("total").and_then(serde_json::Value::as_u64),
            sent_version: v.get("sentVersion").and_then(serde_json::Value::as_i64),
            sent_bytes: v.get("sentBytes").and_then(serde_json::Value::as_u64),
            size_bytes: v.get("sizeBytes").and_then(serde_json::Value::as_u64),
        })
    }

    pub fn poll(path: Option<&str>) -> Option<Reply> {
        let token = token()?;
        parse(&http("GET", &format!("/native/status?path={}", encode(path.unwrap_or(""))), &token, 8)?)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn encodes_a_windows_path_for_a_query() {
            assert_eq!(encode(r"C:\Users\x y\Loja do Zé\Home.figcraft"), "C%3A%5CUsers%5Cx%20y%5CLoja%20do%20Z%C3%A9%5CHome.figcraft");
        }

        #[test]
        fn parses_the_programs_answer() {
            assert_eq!(parse(r#"{"email":"a@b","state":"synced","detail":"versão 4"}"#), Some(Reply { state: "synced".into(), detail: "versão 4".into(), online: true, ..Default::default() }));
            assert_eq!(parse(r#"{"state":"pending"}"#), Some(Reply { state: "pending".into(), detail: String::new(), online: true, ..Default::default() }));
            assert_eq!(parse(r#"{"state":"pending","online":false}"#).map(|r| r.online), Some(false));
            assert_eq!(parse("não é json"), None);
            assert_eq!(parse(r#"{"detail":"x"}"#), None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(state: &str, detail: &str) -> Reply {
        Reply { state: state.into(), detail: detail.into(), online: true, ..Default::default() }
    }

    fn offline(state: &str) -> Reply {
        Reply { state: state.into(), detail: String::new(), online: false, ..Default::default() }
    }

    #[test]
    fn a_synced_file_says_so_and_names_the_version() {
        let p = primary(&reply("synced", "versão 4"), false, true);
        assert_eq!((p.label.as_str(), p.disabled), ("Salvo na LegatIA", true));
        assert!(p.tip.contains("versão 4"));
    }

    #[test]
    fn unsaved_changes_ask_to_save_whatever_the_cloud_says() {
        for state in ["synced", "pending", "behind", "uploading"] {
            let p = primary(&reply(state, ""), true, true);
            assert_eq!((p.id, p.label.as_str(), p.disabled), ("save", "Salvar na LegatIA", false), "{state}");
        }
    }

    #[test]
    fn saved_on_disk_but_not_sent_offers_to_send() {
        assert_eq!(primary(&reply("pending", ""), false, true).label, "Enviar à LegatIA");
        assert_eq!(primary(&reply("new", ""), false, true).label, "Novo · enviar");
        assert_eq!(primary(&reply("pending", ""), false, true).id, "save");
    }

    #[test]
    fn without_a_connection_saving_is_saving_on_this_pc_and_the_rest_waits() {
        // Alteração ainda não salva: o botão salva neste PC
        let p = primary(&offline("synced"), true, true);
        assert_eq!((p.id, p.label.as_str(), p.disabled), ("save", "Salvar neste PC", false));
        // Arquivo novo ou fora da pasta: o quadro de projeto e nome, que também diz "neste PC"
        assert_eq!(primary(&offline("none"), true, false).label, "Salvar neste PC");
        assert_eq!(primary(&offline("outside"), false, true).label, "Salvar neste PC");
        // Já salvo no disco e ainda não enviado: aguarda a conexão (nada a clicar)
        for state in ["pending", "new", "uploading"] {
            let p = primary(&offline(state), false, true);
            assert_eq!((p.id, p.label.as_str(), p.disabled), ("", "Aguardando rede", true), "{state}");
        }
        // O que já está igual à nuvem continua dizendo isso, e sem conta ligada o botão é o de conectar
        assert_eq!(primary(&offline("synced"), false, true).label, "Salvo na LegatIA");
        assert_eq!(primary(&offline("unpaired"), false, true).id, "connect");
        // Com conexão nada muda
        assert_eq!(primary(&reply("pending", ""), false, true).label, "Enviar à LegatIA");
    }

    #[test]
    fn a_big_upload_shows_its_progress_in_the_button() {
        let r = Reply { sent: Some(42 * 1_048_576), total: Some(100 * 1_048_576), ..reply("uploading", "42 MB de 100 MB") };
        let p = primary(&r, false, true);
        assert_eq!((p.label.as_str(), p.busy, p.progress), ("Enviando 42%", true, Some(0.42)));
        assert!(p.tip.contains("42 MB de 100 MB"), "{}", p.tip);
    }

    #[test]
    fn too_large_says_the_size_and_lets_you_try_again() {
        let r = Reply { size_bytes: Some(178_511_687), ..reply("too_large", "170 MB") };
        let p = primary(&r, false, true);
        assert_eq!((p.id, p.label.as_str(), p.disabled), ("save", "Grande demais · tentar", false));
        assert!(p.tip.contains("170 MB"), "{}", p.tip);
    }

    #[test]
    fn a_finished_upload_is_confirmed_once_and_a_refusal_explains_itself() {
        let sending = Reply { sent: Some(5), total: Some(10), ..reply("uploading", "") };
        let done = Reply { sent_version: Some(7), sent_bytes: Some(178_511_687), ..reply("synced", "versão 7") };
        assert_eq!(notice(Some(&sending), &done).as_deref(), Some("Enviado e confirmado na LegatIA: versão 7 · 170 MB."));
        // E o botão confirma também (o aviso pode ser coberto por outro)
        let p = primary(&done, false, true);
        assert_eq!((p.label.as_str(), p.progress), ("Enviado ✓ versão 7", Some(1.0)));
        assert!(p.tip.contains("170 MB"));
        // Já confirmado: não repete
        assert_eq!(notice(Some(&done), &done), None);
        // Abriu um arquivo já igual à nuvem: nada a dizer
        assert_eq!(notice(None, &done), None);
        let big = Reply { size_bytes: Some(178_511_687), ..reply("too_large", "170 MB") };
        assert!(notice(Some(&sending), &big).is_some_and(|m| m.contains("170 MB")));
        assert_eq!(notice(Some(&big), &big), None);
        assert_eq!(mb(4_404_019), "4,2 MB");
    }

    #[test]
    fn uploading_is_busy_and_not_clickable() {
        let p = primary(&reply("uploading", ""), false, true);
        assert!(p.busy && p.id.is_empty());
    }

    #[test]
    fn someone_else_editing_shows_their_name_and_blocks() {
        let p = primary(&reply("locked", "Maria"), false, true);
        assert_eq!((p.label.as_str(), p.disabled), ("Editando: Maria", true));
        assert!(primary(&reply("conflict", ""), false, true).disabled);
    }

    #[test]
    fn a_file_outside_the_folder_is_quiet_and_a_new_document_asks_to_be_saved_in_it() {
        // Fora da pasta o botão leva o arquivo para a LegatIA pelo quadro de projeto e nome
        let outside = primary(&reply("outside", ""), false, true);
        assert_eq!((outside.id, outside.label.as_str(), outside.disabled), ("save", "Salvar na LegatIA", false));
        assert!(primary(&reply("missing", ""), false, true).disabled);
        let p = primary(&reply("none", ""), true, false);
        assert_eq!((p.id, p.label.as_str()), ("save", "Salvar na LegatIA"));
    }

    #[test]
    fn a_long_name_is_cut_so_the_button_stays_short() {
        assert_eq!(primary(&reply("locked", "Maria Fernanda Albuquerque"), false, true).label, "Editando: Maria Fernan…");
    }

    #[test]
    fn before_pairing_the_button_points_to_the_site() {
        let p = primary(&reply("unpaired", ""), false, true);
        assert_eq!((p.id, p.label.as_str(), p.disabled), ("connect", "Conectar à LegatIA", false));
    }
}
