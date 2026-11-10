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
const SITE: &str = "https://app.legatia.com.br/dashboard/design";

/// O que o programa respondeu sobre o arquivo aberto.
#[derive(Clone, Debug, PartialEq)]
pub struct Reply {
    pub state: String,
    pub detail: String,
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
}

struct Shared {
    /// O caminho do arquivo aberto agora, que o fio de consulta manda ao programa.
    path: Option<String>,
    /// `None`: o programa não está rodando (ou o segredo não existe), então o botão do app fica como o do FigCraft.
    reply: Option<Reply>,
    /// A consulta ao programa só existe no desktop.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    started: bool,
}

static SHARED: Mutex<Shared> = Mutex::new(Shared { path: None, reply: None, started: false });

fn shared() -> MutexGuard<'static, Shared> {
    SHARED.lock().unwrap_or_else(|e| e.into_inner())
}

/// O botão para este arquivo: `dirty` é "tem alteração ainda não salva no disco".
pub fn primary(reply: &Reply, dirty: bool, has_path: bool) -> Primary {
    let p = |id: &'static str, label: &str, tip: &str, busy: bool, disabled: bool| Primary { id, label: label.to_string(), tip: tip.to_string(), busy, disabled };
    if reply.state == "unpaired" {
        return p("connect", "Conectar à LegatIA", "Abre a página Design do site: clique em “App no computador” para ligar este app à sua conta.", false, false);
    }
    if !has_path {
        return p("save", "Salvar na LegatIA", "Salve o arquivo dentro da pasta LegatIA Design para ele ir para a nuvem.", false, false);
    }
    if matches!(reply.state.as_str(), "outside" | "missing" | "none") {
        return p("", "Só neste PC", "Este arquivo está fora da pasta LegatIA Design e não vai para a nuvem.", false, true);
    }
    if dirty {
        return p("save", "Salvar na LegatIA", "Salva e envia para a nuvem agora.", false, false);
    }
    match reply.state.as_str() {
        "uploading" => p("", "Enviando…", "Enviando a versão salva para a nuvem.", true, false),
        "pending" => p("save", "Enviar à LegatIA", "Salvo no disco e ainda não enviado. Clique para enviar agora.", false, false),
        "new" => p("save", "Novo · enviar", "Arquivo novo na pasta: clique para criar na nuvem agora.", false, false),
        "locked" => Primary {
            id: "",
            label: format!("Editando: {}", short(&reply.detail)),
            tip: "Outra pessoa está editando. O envio acontece quando a trava soltar (até 15 min).".into(),
            busy: false,
            disabled: true,
        },
        "conflict" => p("", "Conflito", "A nuvem mudou junto: a sua cópia está na pasta _conflitos e este arquivo é o da nuvem.", false, true),
        "behind" => p("", "Nuvem mais nova", "Feche e abra o arquivo para ver a versão da nuvem.", false, true),
        "synced" => Primary { id: "", label: "Salvo na LegatIA".into(), tip: format!("Igual à nuvem · {}.", reply.detail), busy: false, disabled: true },
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
    app.session.docs.get(app.session.active).and_then(|d| d.path.clone())
}

fn current_primary(app: &FigApp) -> Option<Primary> {
    let reply = shared().reply.clone()?;
    let ds = app.session.docs.get(app.session.active)?;
    Some(primary(&reply, ds.is_dirty(), ds.path.is_some()))
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
        host["primary"] = serde_json::json!({ "id": p.id, "label": p.label, "tip": p.tip, "busy": p.busy, "disabled": p.disabled });
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
    app.save();
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(path) = active_path(app) {
        let ctx = ctx.clone();
        let _ = std::thread::Builder::new().name("legatia-sync-now".into()).spawn(move || {
            if let Some(token) = native::token() {
                let _ = native::http("POST", &format!("/native/sync-now?path={}", native::encode(&path)), &token, 180);
            }
            refresh(Some(&path), &ctx);
        });
    }
    #[cfg(target_arch = "wasm32")]
    let _ = ctx;
}

#[cfg(not(target_arch = "wasm32"))]
fn refresh(path: Option<&str>, ctx: &egui::Context) {
    let reply = native::poll(path);
    let changed = {
        let mut s = shared();
        let changed = s.reply != reply;
        s.reply = reply;
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

    pub fn parse(body: &str) -> Option<Reply> {
        let v: serde_json::Value = serde_json::from_str(body).ok()?;
        Some(Reply { state: v.get("state")?.as_str()?.to_string(), detail: v.get("detail").and_then(|d| d.as_str()).unwrap_or("").to_string() })
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
            assert_eq!(parse(r#"{"email":"a@b","state":"synced","detail":"versão 4"}"#), Some(Reply { state: "synced".into(), detail: "versão 4".into() }));
            assert_eq!(parse(r#"{"state":"pending"}"#), Some(Reply { state: "pending".into(), detail: String::new() }));
            assert_eq!(parse("não é json"), None);
            assert_eq!(parse(r#"{"detail":"x"}"#), None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(state: &str, detail: &str) -> Reply {
        Reply { state: state.into(), detail: detail.into() }
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
        assert!(primary(&reply("outside", ""), false, true).disabled);
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
