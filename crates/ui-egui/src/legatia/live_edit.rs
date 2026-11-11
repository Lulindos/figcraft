//! LegatIA: editar ao vivo com a equipe.
//!
//! O app não fala com o servidor ao vivo nem com o Supabase: pede ao programa `legatia-sync` (`/native/live`), que pergunta ao
//! porteiro (live.legatia.com.br) com a sessão da pessoa e devolve o link só dela. Aqui o link vira uma aba de sessão ao vivo
//! (`collab.join`, o mesmo caminho de qualquer sessão do FigCraft). Enquanto a aba existe, o app avisa o programa de minuto em minuto
//! (`/native/live-ping`); ele mantém o "ainda estou aqui" ao porteiro. Se a permissão mudou ou a pessoa saiu do projeto, o porteiro
//! derruba a sessão e o próximo aviso traz o motivo.
//!
//! O arquivo da sessão nasce da última versão salva na LegatIA, então só começa com o arquivo aberto salvo e enviado. Quem salva na aba
//! ao vivo ("Salvar na LegatIA") grava o desenho no mesmo arquivo da pasta, de onde o programa o envia como qualquer versão.
//!
//! Nada disto existe no navegador (lá é a página hospedeira).

use std::sync::{Mutex, MutexGuard};

use crate::FigApp;

/// O que o menu das pessoas oferece para o arquivo aberto.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Offer {
    /// Nada (sem arquivo da pasta aberto, ou já em outra sessão que não é da LegatIA).
    None,
    Start,
    Leave,
}

/// O que a pessoa pediu no menu, para o próximo quadro (o menu só enxerga o app sem poder mudá-lo).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Request {
    Start,
    Leave,
}

#[derive(Default)]
struct State {
    request: Option<Request>,
    /// Uma consulta ao programa está andando (não começar duas).
    busy: bool,
    /// O arquivo da pasta de onde a sessão nasceu: é onde "Salvar na LegatIA" grava quando a aba é a ao vivo.
    origin: Option<String>,
    /// O que o fio de consulta trouxe, para o app tratar no próximo quadro.
    outcome: Option<Outcome>,
    /// O último aviso ao programa e o último quadro em que uma aba de sessão ao vivo existia.
    last_ping: Option<std::time::Instant>,
    tab_seen: Option<std::time::Instant>,
}

#[derive(Debug)]
enum Outcome {
    /// O programa devolveu o link: `{link, access, name, desatualizado}` e o arquivo de onde veio.
    Link { path: String, index: usize, body: serde_json::Value },
    /// Uma mensagem para a pessoa (não deu para começar).
    Said(String),
    /// A sessão caiu pelo porteiro (permissão mudou, saiu do projeto): o motivo.
    Ended(String),
}

static STATE: Mutex<State> = Mutex::new(State { request: None, busy: false, origin: None, outcome: None, last_ping: None, tab_seen: None });

fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// O que o menu das pessoas oferece agora para o arquivo aberto.
pub fn offer(app: &FigApp) -> Offer {
    if cfg!(any(test, target_arch = "wasm32")) {
        return Offer::None;
    }
    let active = state().origin.is_some() && in_live_tab(app);
    if active {
        return Offer::Leave;
    }
    let Some(ds) = app.session.docs.get(app.session.active) else { return Offer::None };
    // Só um arquivo da pasta (com caminho) e sem sessão ao vivo nesta aba
    if ds.path.is_some() && crate::tabs::live_mark(app, ds.uid).is_none() {
        Offer::Start
    } else {
        Offer::None
    }
}

/// Alguma aba está numa sessão ao vivo (a que este módulo abriu).
fn in_live_tab(app: &FigApp) -> bool {
    app.session.docs.iter().any(|d| crate::tabs::live_mark(app, d.uid).is_some())
}

/// O menu das pessoas pediu para começar ou sair.
pub fn request_start() {
    state().request = Some(Request::Start);
}

pub fn request_leave() {
    state().request = Some(Request::Leave);
}

/// O arquivo da pasta de onde a sessão ao vivo nasceu, quando a aba aberta é a da sessão (para "Salvar na LegatIA" gravar nele).
pub fn origin_for_save(app: &FigApp) -> Option<String> {
    let origin = state().origin.clone()?;
    let ds = app.session.docs.get(app.session.active)?;
    (ds.path.is_none() && crate::tabs::live_mark(app, ds.uid).is_some()).then_some(origin)
}

/// Grava o desenho da aba ao vivo no arquivo da pasta e manda o programa enviá-lo agora.
#[cfg(not(target_arch = "wasm32"))]
pub fn save_to_origin(app: &mut FigApp, ctx: &egui::Context, origin: String) {
    if app.run_user("file.saveAs", serde_json::json!({"path": origin})).is_some() {
        super::sync::send_now(origin, ctx);
        app.toast("Salvo no arquivo da pasta. Indo para a LegatIA.");
    }
}

/// A cada quadro: trata o pedido do menu, o que o fio trouxe e o aviso periódico ao programa.
#[cfg(not(target_arch = "wasm32"))]
pub fn frame(app: &mut FigApp, ctx: &egui::Context) {
    if cfg!(test) {
        return;
    }
    let (request, outcome) = {
        let mut s = state();
        (s.request.take(), s.outcome.take())
    };
    match request {
        Some(Request::Start) => start(app, ctx),
        Some(Request::Leave) => leave(app, ctx),
        None => {}
    }
    match outcome {
        Some(Outcome::Link { path, index, body }) => joined(app, path, index, &body),
        Some(Outcome::Said(msg)) => app.toast(msg),
        Some(Outcome::Ended(msg)) => {
            app.toast(msg);
            // Sai da sessão (a aba vira uma cópia sua) e para de avisar o programa
            if in_live_tab(app) {
                app.run("collab.leave", serde_json::json!({}));
            }
            state().origin = None;
        }
        None => {}
    }
    keep_alive(app, ctx);
}

#[cfg(target_arch = "wasm32")]
pub fn frame(_app: &mut FigApp, _ctx: &egui::Context) {}

#[cfg(not(target_arch = "wasm32"))]
fn start(app: &mut FigApp, ctx: &egui::Context) {
    let index = app.session.active;
    let Some(ds) = app.session.docs.get(index) else { return };
    let Some(path) = ds.path.clone() else {
        app.toast("Salve o arquivo na pasta da LegatIA antes de editar ao vivo.");
        return;
    };
    if ds.is_dirty() {
        app.toast("Salve o arquivo antes (Salvar na LegatIA): a sessão ao vivo começa da última versão salva.");
        return;
    }
    {
        let mut s = state();
        if s.busy {
            return;
        }
        s.busy = true;
    }
    app.toast("Abrindo a sessão ao vivo…");
    let ctx = ctx.clone();
    let _ = std::thread::Builder::new().name("legatia-live-start".into()).spawn(move || {
        let outcome = match super::sync::post(&format!("/native/live?path={}", super::sync::encode_query(&path)), 150) {
            None => Outcome::Said("O programa de sincronização não respondeu. Ele precisa estar rodando.".into()),
            Some((200, body)) => match serde_json::from_str::<serde_json::Value>(&body) {
                Ok(v) if v.get("link").and_then(|l| l.as_str()).is_some() => Outcome::Link { path, index, body: v },
                _ => Outcome::Said("Resposta inesperada do servidor ao vivo.".into()),
            },
            Some((_, body)) => Outcome::Said(message_of(&body)),
        };
        let mut s = state();
        s.busy = false;
        s.outcome = Some(outcome);
        drop(s);
        ctx.request_repaint();
    });
}

/// A mensagem de um erro do programa (`{error, message}`), ou um texto geral.
fn message_of(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(str::to_string))
        .unwrap_or_else(|| "Não consegui abrir a sessão ao vivo.".to_string())
}

#[cfg(not(target_arch = "wasm32"))]
fn joined(app: &mut FigApp, path: String, index: usize, body: &serde_json::Value) {
    let link = body.get("link").and_then(|l| l.as_str()).unwrap_or_default().to_string();
    let access = body.get("access").and_then(|a| a.as_str()).unwrap_or("view");
    let name = body.get("name").and_then(|n| n.as_str()).unwrap_or("arquivo").to_string();
    // Quem está na sessão aparece com o nome da conta da LegatIA (e não "Guest 1"), se a pessoa ainda não escolheu um nome
    if let Some(overview) = super::home::current() {
        let me = overview.user.name.trim();
        if !me.is_empty() && app.session.prefs.user_name == figcraft_engine::cmd::comment::DEFAULT_AUTHOR {
            app.session.prefs.user_name = me.to_string();
        }
    }
    if app.run_user("collab.join", serde_json::json!({"link": link})).is_none() {
        // O link foi dado e não abriu: libera a vaga no porteiro
        stop_remote(path);
        return;
    }
    // A aba de onde a sessão nasceu sai (a da sessão toma o lugar dela): duas abas do mesmo arquivo, uma velha, só dariam conflito.
    // Só se ela segue igual ao arquivo da pasta (sem alteração nova) e é a mesma.
    if app.session.docs.get(index).is_some_and(|d| d.path.as_deref() == Some(path.as_str()) && !d.is_dirty()) {
        app.run("file.close", serde_json::json!({"index": index}));
    }
    {
        let mut s = state();
        s.origin = Some(path);
        s.last_ping = Some(std::time::Instant::now());
        s.tab_seen = Some(std::time::Instant::now());
    }
    let mut msg = if access == "edit" { format!("Ao vivo em “{name}”. Salvar na LegatIA grava o desenho no arquivo da pasta.") } else { format!("Ao vivo em “{name}”, só para ver: você não edita este arquivo.") };
    if body.get("desatualizado").and_then(|d| d.as_bool()) == Some(true) {
        msg.push_str(" Atenção: a LegatIA tem uma versão mais nova do que a da sessão.");
    }
    app.toast(msg);
}

#[cfg(not(target_arch = "wasm32"))]
fn leave(app: &mut FigApp, _ctx: &egui::Context) {
    if in_live_tab(app) {
        app.run("collab.leave", serde_json::json!({}));
    }
    if let Some(path) = state().origin.take() {
        stop_remote(path);
    }
    app.toast("Você saiu da sessão ao vivo. A aba ficou como uma cópia sua.");
}

/// Avisa o programa que a sessão acabou (o porteiro revoga o link).
#[cfg(not(target_arch = "wasm32"))]
fn stop_remote(path: String) {
    let _ = std::thread::Builder::new().name("legatia-live-stop".into()).spawn(move || {
        let _ = super::sync::post(&format!("/native/live-stop?path={}", super::sync::encode_query(&path)), 25);
    });
}

/// De minuto em minuto, enquanto a aba da sessão existe: diz ao programa que segue aberta. Se a aba sumiu, encerra no porteiro.
#[cfg(not(target_arch = "wasm32"))]
fn keep_alive(app: &mut FigApp, ctx: &egui::Context) {
    const EVERY: std::time::Duration = std::time::Duration::from_secs(60);
    const GONE_AFTER: std::time::Duration = std::time::Duration::from_secs(6);
    let origin = state().origin.clone();
    let Some(origin) = origin else { return };
    let now = std::time::Instant::now();
    if in_live_tab(app) {
        state().tab_seen = Some(now);
    } else if state().tab_seen.is_some_and(|t| now.duration_since(t) > GONE_AFTER) {
        // A aba foi fechada: a pessoa saiu da sessão sem o menu
        state().origin = None;
        stop_remote(origin);
        return;
    }
    let due = state().last_ping.is_none_or(|t| now.duration_since(t) >= EVERY);
    if !due {
        // Acorda de tempos em tempos mesmo sem mexer no mouse
        ctx.request_repaint_after(EVERY);
        return;
    }
    state().last_ping = Some(now);
    let ctx = ctx.clone();
    let _ = std::thread::Builder::new().name("legatia-live-ping".into()).spawn(move || {
        if let Some((200, body)) = super::sync::post(&format!("/native/live-ping?path={}", super::sync::encode_query(&origin)), 20) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
                if v.get("active").and_then(|a| a.as_bool()) == Some(false) {
                    let why = v.pointer("/ended/message").and_then(|m| m.as_str()).unwrap_or("A sessão ao vivo foi encerrada.").to_string();
                    state().outcome = Some(Outcome::Ended(why));
                    ctx.request_repaint();
                }
            }
        }
    });
}

/// O item do menu das pessoas (o chevron ao lado das fotos): começar ou sair da sessão ao vivo. `true`: foi clicado.
pub fn people_menu_item(ui: &mut egui::Ui, app: &FigApp) -> bool {
    let (label, leave) = match offer(app) {
        Offer::None => return false,
        Offer::Start => ("Editar ao vivo com a equipe", false),
        Offer::Leave => ("Sair da sessão ao vivo", true),
    };
    ui.separator();
    if crate::right::menu_item(ui, label, "") {
        if leave {
            request_leave();
        } else {
            request_start();
        }
        ui.close();
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_error_answer_gives_its_message() {
        assert_eq!(message_of(r#"{"error":"estado","message":"Envie o arquivo antes."}"#), "Envie o arquivo antes.");
        assert_eq!(message_of("não é json"), "Não consegui abrir a sessão ao vivo.");
        assert_eq!(message_of(r#"{"error":"x"}"#), "Não consegui abrir a sessão ao vivo.");
    }

    #[test]
    fn the_menu_offers_nothing_in_tests_and_the_request_waits_for_the_next_frame() {
        request_start();
        assert_eq!(state().request, Some(Request::Start));
        request_leave();
        assert_eq!(state().request, Some(Request::Leave));
        state().request = None;
    }
}
