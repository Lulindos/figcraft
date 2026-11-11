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
//! Desde a 0.17.10 o ao vivo liga sozinho: abrir um arquivo da pasta que está igual à nuvem já entra na sessão (sem o menu). E a sessão
//! se salva sozinha: um só participante grava na LegatIA (o que entrou primeiro, menor número de cliente da sessão), alguns segundos
//! depois que a mexida para ou no máximo a cada 3 minutos, nunca mais de uma vez a cada 2 minutos. Os outros não gravam (seriam versões
//! repetidas disputando a mesma trava); "Salvar na LegatIA" deles só avisa quem está gravando.
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
    /// Ctrl+S na aba ao vivo (o salvar do app não tem o contexto da janela): grava no próximo quadro.
    Save,
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
    /// A pessoa pode editar (o porteiro deu `edit`): só quem edita grava.
    can_edit: bool,
    /// Os arquivos que já tentaram entrar sozinhos nesta execução (não tentar de novo a cada quadro, nem voltar depois de sair).
    auto_tried: Vec<String>,
    /// O pedido em andamento foi automático (um erro vira um aviso curto, não um bloqueio).
    auto: bool,
    /// Já avisou que o app não está ligado à conta.
    unpaired_said: bool,
    /// O salvamento automático da sessão.
    saver: Saver,
}

/// O salvamento automático: quando o desenho mudou (a primeira mudança ainda não salva e a última) e quando foi a última gravação.
#[derive(Default)]
struct Saver {
    doc: usize,
    first_change: Option<std::time::Instant>,
    last_change: Option<std::time::Instant>,
    saved_at: Option<std::time::Instant>,
    /// Quem grava a sessão, visto da última vez que a aba ao vivo estava na frente: `None` sou eu.
    saver_name: Option<String>,
}

impl Saver {
    const fn new() -> Self {
        Saver { doc: 0, first_change: None, last_change: None, saved_at: None, saver_name: None }
    }
}

/// Depois de quanto tempo parado o desenho é gravado, o máximo que uma mudança espera e o intervalo mínimo entre gravações.
pub const SAVE_QUIET_SECS: u64 = 20;
pub const SAVE_MAX_WAIT_SECS: u64 = 180;
pub const SAVE_MIN_GAP_SECS: u64 = 120;

/// Se está na hora de gravar: houve mudança e (parou há `SAVE_QUIET_SECS`, ou a primeira já espera `SAVE_MAX_WAIT_SECS`), e a última
/// gravação foi há pelo menos `SAVE_MIN_GAP_SECS`. Os tempos são em segundos desde cada marco.
pub fn save_due(since_first: Option<u64>, since_last_change: Option<u64>, since_saved: Option<u64>) -> bool {
    let (Some(first), Some(last)) = (since_first, since_last_change) else { return false };
    let waited = last >= SAVE_QUIET_SECS || first >= SAVE_MAX_WAIT_SECS;
    waited && since_saved.is_none_or(|s| s >= SAVE_MIN_GAP_SECS)
}

/// Quem grava a sessão: o participante de menor número (o primeiro a entrar). `me` é o meu número; `peers`, os dos outros com nome.
/// `None`: sou eu; senão o nome de quem grava.
pub fn saver_among(me: Option<u64>, peers: &[(u64, String)]) -> Option<String> {
    let me = me?;
    peers.iter().filter(|(id, _)| *id < me).min_by_key(|(id, _)| *id).map(|(_, name)| name.clone())
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

static STATE: Mutex<State> = Mutex::new(State {
    request: None,
    busy: false,
    origin: None,
    outcome: None,
    last_ping: None,
    tab_seen: None,
    can_edit: false,
    auto_tried: Vec::new(),
    auto: false,
    unpaired_said: false,
    saver: Saver::new(),
});

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

/// O salvar do app (Ctrl+S, menu Arquivo) na aba ao vivo: grava no arquivo da pasta no próximo quadro. `true`: era a aba ao vivo.
pub fn request_save(app: &FigApp) -> bool {
    if cfg!(any(test, target_arch = "wasm32")) || origin_for_save(app).is_none() {
        return false;
    }
    state().request = Some(Request::Save);
    true
}

/// O arquivo da pasta de onde a sessão ao vivo nasceu, quando a aba aberta é a da sessão (para "Salvar na LegatIA" gravar nele).
pub fn origin_for_save(app: &FigApp) -> Option<String> {
    let origin = state().origin.clone()?;
    let ds = app.session.docs.get(app.session.active)?;
    crate::tabs::live_mark(app, ds.uid).is_some().then_some(origin)
}

/// A aba da frente é a da sessão ao vivo da LegatIA: o arquivo da pasta de onde ela nasceu (para o botão primário mostrar o estado dele).
pub fn live_origin(app: &FigApp) -> Option<String> {
    origin_for_save(app)
}

/// Quem grava a sessão quando não sou eu (o botão e o "Salvar" mostram o nome). `None`: sou eu ou não há sessão.
pub fn other_saver() -> Option<String> {
    state().saver.saver_name.clone()
}

/// Grava o desenho da aba ao vivo no arquivo da pasta e manda o programa enviá-lo agora.
#[cfg(not(target_arch = "wasm32"))]
pub fn save_to_origin(app: &mut FigApp, ctx: &egui::Context, origin: String) {
    if !state().can_edit {
        app.toast("Você está nesta sessão só para ver: quem edita é que grava na LegatIA.");
        return;
    }
    // Outra pessoa da sessão grava (a primeira que entrou): gravar aqui também só criaria versões repetidas disputando a trava
    if let Some(name) = other_saver() {
        app.toast(format!("A sessão ao vivo grava sozinha na LegatIA ({name} grava a cada poucos minutos). Tudo o que você fez já está na sessão."));
        return;
    }
    if write_origin(app, ctx, &origin) {
        app.toast("Salvo na pasta. Indo para a LegatIA.");
    }
}

/// Grava o desenho da aba ao vivo (a da frente) no arquivo da pasta e manda o programa enviar. `true`: gravou.
#[cfg(not(target_arch = "wasm32"))]
fn write_origin(app: &mut FigApp, ctx: &egui::Context, origin: &str) -> bool {
    if app.run_user("file.saveAs", serde_json::json!({"path": origin})).is_none() {
        return false;
    }
    super::sync::send_now(origin.to_string(), ctx);
    let mut s = state();
    s.saver.first_change = None;
    s.saver.saved_at = Some(std::time::Instant::now());
    true
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
        Some(Request::Start) => {
            state().auto = false;
            start(app, ctx);
        }
        Some(Request::Leave) => leave(app, ctx),
        Some(Request::Save) => {
            if let Some(origin) = origin_for_save(app) {
                save_to_origin(app, ctx, origin);
            }
        }
        None => auto_start(app, ctx),
    }
    match outcome {
        Some(Outcome::Link { path, index, body }) => joined(app, path, index, &body),
        Some(Outcome::Said(msg)) => {
            // Entrou sozinho e não deu: segue editando a cópia da pasta, com o motivo
            if std::mem::take(&mut state().auto) {
                app.toast(format!("Ao vivo indisponível agora: {msg} Você edita a sua cópia e salva normalmente."));
            } else {
                app.toast(msg);
            }
        }
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
    autosave(app, ctx);
}

/// Abriu um arquivo da pasta que está igual à nuvem e não há sessão: entra ao vivo sozinho (uma vez por arquivo nesta execução).
/// Sem o app ligado à conta, avisa uma vez como ligar.
#[cfg(not(target_arch = "wasm32"))]
fn auto_start(app: &mut FigApp, ctx: &egui::Context) {
    if state().origin.is_some() || state().busy {
        return;
    }
    let Some(ds) = app.session.docs.get(app.session.active) else { return };
    let Some(path) = ds.path.clone() else { return };
    if ds.is_dirty() || crate::tabs::live_mark(app, ds.uid).is_some() || in_live_tab(app) {
        return;
    }
    match super::sync::state_for(&path).as_deref() {
        Some("synced") => {
            {
                let mut s = state();
                if s.auto_tried.contains(&path) {
                    return;
                }
                s.auto_tried.push(path);
                s.auto = true;
            }
            start(app, ctx);
        }
        Some("unpaired") => {
            let said = std::mem::replace(&mut state().unpaired_said, true);
            if !said {
                app.toast("Este app não está ligado à sua conta da LegatIA: nada vai para a nuvem e não há edição ao vivo. No site, em Design ▸ “App no computador”, clique em “Conectar à minha conta”.");
            }
        }
        _ => {}
    }
}

/// O salvamento automático da sessão: só na aba ao vivo da frente, só quem edita e só quem grava pela sessão (o primeiro a entrar).
#[cfg(not(target_arch = "wasm32"))]
fn autosave(app: &mut FigApp, ctx: &egui::Context) {
    let Some(origin) = origin_for_save(app) else { return };
    let Some(ds) = app.session.docs.get(app.session.active) else { return };
    let now = std::time::Instant::now();
    let doc = std::sync::Arc::as_ptr(&ds.doc) as usize;
    let dirty = ds.is_dirty();
    // Quem grava: o de menor número na sessão
    let summary = app.session.live_summary();
    let me = summary.get("me").and_then(serde_json::Value::as_u64);
    let peers: Vec<(u64, String)> = summary
        .get("peers")
        .and_then(serde_json::Value::as_array)
        .map(|a| a.iter().filter_map(|p| Some((p.get("id")?.as_u64()?, p.get("name").and_then(|n| n.as_str()).unwrap_or("outra pessoa").to_string()))).collect())
        .unwrap_or_default();
    let due = {
        let mut s = state();
        s.saver.saver_name = saver_among(me, &peers);
        if s.saver.doc != doc {
            if s.saver.doc != 0 && dirty {
                s.saver.first_change.get_or_insert(now);
                s.saver.last_change = Some(now);
            }
            s.saver.doc = doc;
        }
        let secs = |t: Option<std::time::Instant>| t.map(|t| now.duration_since(t).as_secs());
        let mine = s.can_edit && s.saver.saver_name.is_none() && me.is_some();
        mine && dirty && save_due(secs(s.saver.first_change), secs(s.saver.last_change), secs(s.saver.saved_at))
    };
    if due {
        write_origin(app, ctx, &origin);
    } else if dirty {
        ctx.request_repaint_after(std::time::Duration::from_secs(5));
    }
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
        s.can_edit = access == "edit";
        s.auto = false;
        s.saver = Saver::new();
    }
    let mut msg = if access == "edit" { format!("Ao vivo em “{name}”: quem abrir este arquivo vê as mudanças na hora, e a sessão grava na LegatIA sozinha.") } else { format!("Ao vivo em “{name}”, só para ver: você não edita este arquivo.") };
    if body.get("desatualizado").and_then(|d| d.as_bool()) == Some(true) {
        msg.push_str(" Atenção: a LegatIA tem uma versão mais nova do que a da sessão.");
    }
    app.toast(msg);
}

#[cfg(not(target_arch = "wasm32"))]
fn leave(app: &mut FigApp, ctx: &egui::Context) {
    // Quem grava a sessão sai com o que falta salvar já gravado
    if let Some(origin) = origin_for_save(app) {
        let pending = app.session.docs.get(app.session.active).is_some_and(|d| d.is_dirty());
        let mine = { let s = state(); s.can_edit && s.saver.saver_name.is_none() };
        if pending && mine {
            write_origin(app, ctx, &origin);
        }
    }
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
    fn the_session_saves_after_a_pause_or_at_most_every_few_minutes_never_too_often() {
        // Sem mudança, nada
        assert!(!save_due(None, None, None));
        // Mexendo sem parar há pouco: espera
        assert!(!save_due(Some(10), Some(1), None));
        // Parou há 20 s: grava
        assert!(save_due(Some(30), Some(SAVE_QUIET_SECS), None));
        // Mexendo sem parar há 3 min: grava mesmo assim
        assert!(save_due(Some(SAVE_MAX_WAIT_SECS), Some(1), None));
        // Gravou há 1 min: espera completar 2
        assert!(!save_due(Some(200), Some(60), Some(60)));
        assert!(save_due(Some(200), Some(60), Some(SAVE_MIN_GAP_SECS)));
    }

    #[test]
    fn only_the_first_one_in_the_session_saves() {
        let peers = vec![(7, "Ana".to_string()), (12, "Bruno".to_string())];
        // Eu sou o 5: ninguém entrou antes, eu gravo
        assert_eq!(saver_among(Some(5), &peers), None);
        // Eu sou o 9: a Ana (7) grava
        assert_eq!(saver_among(Some(9), &peers), Some("Ana".to_string()));
        // Sozinho na sessão: eu gravo
        assert_eq!(saver_among(Some(3), &[]), None);
        // Ainda sem número (conectando): ninguém diz que é outro
        assert_eq!(saver_among(None, &peers), None);
    }

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
