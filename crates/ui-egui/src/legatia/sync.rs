//! LegatIA: como está o arquivo aberto em relação à LegatIA, e o botão que salva e envia.
//!
//! O app nativo não fala com o Supabase: o programa `legatia-sync` (tools/legatia-sync, no repositório do site)
//! espelha os arquivos da conta numa pasta do computador. Este módulo só pergunta a ele, em 127.0.0.1, como está o
//! arquivo aberto, com o segredo que ele guarda em `%APPDATA%\LegatIA Design\native.token`, e desenha um botão na
//! barra de abas: o estado, e um clique que salva no disco e manda enviar na hora. Sem o programa rodando, nada
//! aparece. Nada disto existe no navegador (lá a página hospedeira salva).

use std::sync::{Mutex, MutexGuard};

use egui::{CornerRadius, Rect, Sense, Stroke, Ui, pos2, vec2};

use crate::FigApp;
use crate::a11y::Tip;
use crate::theme::{Tokens, regular};

/// O que o programa respondeu sobre o arquivo aberto.
#[derive(Clone, Debug, PartialEq)]
pub struct Reply {
    pub state: String,
    pub detail: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tone {
    Ok,
    Busy,
    Warn,
    Bad,
    Quiet,
}

/// O texto e a cor do botão, e se um clique faz alguma coisa.
#[derive(Clone, Debug, PartialEq)]
pub struct Look {
    pub label: String,
    pub tone: Tone,
    pub tip: &'static str,
}

struct Shared {
    /// O caminho do arquivo aberto agora, que o fio de consulta manda ao programa.
    path: Option<String>,
    /// `None`: o programa não está rodando (ou o segredo não existe), então o botão não aparece.
    reply: Option<Reply>,
    /// A consulta ao programa só existe no desktop.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    started: bool,
}

static SHARED: Mutex<Shared> = Mutex::new(Shared { path: None, reply: None, started: false });

fn shared() -> MutexGuard<'static, Shared> {
    SHARED.lock().unwrap_or_else(|e| e.into_inner())
}

/// O botão para este arquivo: `dirty` é "tem alteração ainda não salva no disco". Sem resposta do programa, nada.
pub fn look(reply: &Reply, dirty: bool, has_path: bool) -> Look {
    let (label, tone, tip): (String, Tone, &'static str) = if reply.state == "unpaired" {
        ("LegatIA: conectar".into(), Tone::Quiet, "Abra a página Design no site e clique em “App no computador” para ligar este app à sua conta.")
    } else if !has_path {
        ("Novo · salvar na LegatIA".into(), Tone::Warn, "Salve o arquivo dentro da pasta LegatIA Design para ele ir para a nuvem.")
    } else if matches!(reply.state.as_str(), "outside" | "missing" | "none") {
        ("Só neste computador".into(), Tone::Quiet, "Este arquivo está fora da pasta LegatIA Design e não vai para a nuvem.")
    } else if dirty {
        ("Salvar na LegatIA".into(), Tone::Warn, "Salva e envia para a nuvem agora.")
    } else {
        match reply.state.as_str() {
            "uploading" => ("Enviando…".into(), Tone::Busy, "Enviando a versão salva para a nuvem."),
            "pending" => ("Enviar para a LegatIA".into(), Tone::Warn, "Salvo no disco e ainda não enviado. Clique para enviar agora."),
            "new" => ("Novo · enviar".into(), Tone::Warn, "Arquivo novo na pasta: clique para criar na nuvem agora."),
            "locked" => (format!("Em edição por {}", reply.detail), Tone::Bad, "Outra pessoa está editando. O envio acontece quando a trava soltar (até 15 min)."),
            "conflict" => ("Conflito · veja _conflitos".into(), Tone::Bad, "A nuvem mudou junto: a sua cópia está na pasta _conflitos e este arquivo é o da nuvem."),
            "behind" => ("Nuvem tem versão mais nova".into(), Tone::Warn, "Feche e abra o arquivo para ver a versão da nuvem."),
            "synced" => (format!("Na LegatIA · {}", reply.detail), Tone::Ok, "Igual à nuvem. Clique para salvar e enviar de novo."),
            _ => ("LegatIA".into(), Tone::Quiet, "Estado desconhecido."),
        }
    };
    Look { label, tone, tip }
}

fn active_path(app: &FigApp) -> Option<String> {
    app.session.docs.get(app.session.active).and_then(|d| d.path.clone())
}

fn current_look(app: &FigApp) -> Option<Look> {
    let reply = shared().reply.clone()?;
    let ds = app.session.docs.get(app.session.active)?;
    Some(look(&reply, ds.is_dirty(), ds.path.is_some()))
}

fn width_of(label: &str) -> f32 {
    (label.chars().count() as f32 * 6.4 + 34.0).clamp(96.0, 260.0)
}

/// A largura que o botão toma da barra de abas neste quadro (0 sem o programa); também liga a consulta ao programa.
pub fn reserved(app: &FigApp, ctx: &egui::Context) -> f32 {
    ensure_started(ctx);
    shared().path = active_path(app);
    current_look(app).map_or(0.0, |l| width_of(&l.label) + 8.0)
}

/// Desenha o botão à esquerda de `left_of` e devolve o retângulo que ele ocupou (ou `left_of`, sem o programa).
pub fn chip(app: &mut FigApp, ui: &mut Ui, t: &Tokens, left_of: Rect) -> Rect {
    let Some(look) = current_look(app) else { return left_of };
    let w = width_of(&look.label);
    let r = Rect::from_center_size(pos2(left_of.min.x - 8.0 - w / 2.0, left_of.center().y), vec2(w, 24.0));
    let clickable = look.tone != Tone::Quiet || !look.label.starts_with("Só");
    let resp = ui.interact(r, ui.scope_id().with("legatia_sync_chip"), if clickable { Sense::click() } else { Sense::hover() });
    let resp = resp.tip(look.tip);
    if resp.hovered() && clickable {
        ui.painter().rect_filled(r, CornerRadius::same(12), t.hover);
    }
    ui.painter().rect_stroke(r, CornerRadius::same(12), Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    let dot = match look.tone {
        Tone::Ok => t.success,
        Tone::Busy => crate::theme::ACCENT,
        Tone::Warn => t.warning,
        Tone::Bad => t.danger,
        Tone::Quiet => t.icon_secondary,
    };
    ui.painter().circle_filled(pos2(r.min.x + 12.0, r.center().y), 3.5, dot);
    ui.painter().text(
        pos2(r.min.x + 22.0, r.center().y),
        egui::Align2::LEFT_CENTER,
        &look.label,
        regular(11.5),
        if resp.hovered() { t.text } else { t.text_secondary },
    );
    if resp.clicked() && clickable {
        save_now(app, ui.ctx());
    }
    r
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
        let l = look(&reply("synced", "versão 4"), false, true);
        assert_eq!((l.label.as_str(), l.tone), ("Na LegatIA · versão 4", Tone::Ok));
    }

    #[test]
    fn unsaved_changes_ask_to_save_whatever_the_cloud_says() {
        for state in ["synced", "pending", "behind", "uploading"] {
            let l = look(&reply(state, ""), true, true);
            assert_eq!((l.label.as_str(), l.tone), ("Salvar na LegatIA", Tone::Warn), "{state}");
        }
    }

    #[test]
    fn saved_on_disk_but_not_sent_offers_to_send() {
        assert_eq!(look(&reply("pending", ""), false, true).label, "Enviar para a LegatIA");
        assert_eq!(look(&reply("new", ""), false, true).label, "Novo · enviar");
    }

    #[test]
    fn someone_else_editing_shows_their_name_as_a_problem() {
        let l = look(&reply("locked", "Maria"), false, true);
        assert_eq!((l.label.as_str(), l.tone), ("Em edição por Maria", Tone::Bad));
        assert_eq!(look(&reply("conflict", ""), false, true).tone, Tone::Bad);
    }

    #[test]
    fn a_file_outside_the_folder_is_quiet_and_a_new_document_asks_to_be_saved_in_it() {
        assert_eq!(look(&reply("outside", ""), false, true).tone, Tone::Quiet);
        assert_eq!(look(&reply("none", ""), true, false).label, "Novo · salvar na LegatIA");
    }

    #[test]
    fn before_pairing_the_button_points_to_the_site() {
        let l = look(&reply("unpaired", ""), false, true);
        assert_eq!((l.label.as_str(), l.tone), ("LegatIA: conectar", Tone::Quiet));
    }

    #[test]
    fn the_button_grows_with_its_text_within_limits() {
        assert!(width_of("Enviando…") < width_of("Em edição por Maria Fernanda"));
        assert_eq!(width_of(""), 96.0);
        assert_eq!(width_of(&"x".repeat(200)), 260.0);
    }
}
