//! LegatIA: "Atualizar". Quando o arquivo da pasta muda debaixo de uma aba aberta (a LegatIA tinha uma versão mais nova, ou
//! outro programa salvou), o editor já se recusa a salvar por cima. Aqui isso vira um botão: o que a pessoa mudou na aba vai
//! para `_conflitos`, o programa de sincronização deixa a pasta na versão da nuvem e a aba relê o arquivo. Nada é perdido.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::FigApp;

/// De quanto em quanto tempo olhar o arquivo no disco (o botão pergunta a cada quadro).
const LOOK_EVERY: Duration = Duration::from_millis(700);

/// O que o programa respondeu ao pedido de atualizar.
struct Done {
    path: String,
    /// `Ok`: a pasta está na versão da nuvem (e o que ele guardou em `_conflitos`, se guardou). `Err`: o motivo.
    result: Result<Option<String>, String>,
    /// A cópia da aba, feita aqui antes de pedir.
    mine: Option<PathBuf>,
}

#[derive(Default)]
struct State {
    /// (aba, quando olhei, mudou?)
    looked: Option<(u64, Instant, bool)>,
    running: bool,
    done: Option<Done>,
}

static STATE: Mutex<State> = Mutex::new(State { looked: None, running: false, done: None });

fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// O arquivo da aba da frente mudou no disco desde que ela o leu ou gravou?
pub fn stale(app: &FigApp) -> bool {
    let Some(ds) = app.session.docs.get(app.session.active) else { return false };
    let now = Instant::now();
    let mut s = state();
    if let Some((uid, at, changed)) = s.looked
        && uid == ds.uid
        && now.duration_since(at) < LOOK_EVERY
    {
        return changed;
    }
    let changed = ds.changed_on_disk();
    s.looked = Some((ds.uid, now, changed));
    changed
}

/// Um pedido de atualizar está andando.
pub fn running() -> bool {
    state().running
}

/// Onde guardar a cópia da aba de `path` (`<pasta da LegatIA>/<projeto>/<nome>.figcraft`), sem passar por cima de outra cópia.
/// `when`: a data e a hora, para o nome.
pub fn copy_path(path: &Path, when: &str, taken: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    let folder = path.parent()?;
    let root = folder.parent()?;
    let project = folder.file_name()?.to_string_lossy();
    let name = path.file_stem()?.to_string_lossy();
    let dir = root.join("_conflitos");
    let base = format!("{project} - {name} (minha cópia {when})");
    (1..100).map(|n| if n == 1 { dir.join(format!("{base}.figcraft")) } else { dir.join(format!("{base} ({n}).figcraft")) }).find(|p| !taken(p))
}

/// Grava o que está na aba (com as alterações ainda não salvas) numa cópia em `_conflitos`.
#[cfg(not(target_arch = "wasm32"))]
fn keep_mine(app: &mut FigApp, path: &str) -> Result<PathBuf, String> {
    use base64::Engine as _;
    let data = app.run("file.saveAs", serde_json::json!({"data": true})).and_then(|r| r.get("data").and_then(|d| d.as_str()).map(str::to_string));
    let data = data.ok_or("não consegui gerar a cópia")?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(data).map_err(|e| e.to_string())?;
    let when = chrono::Local::now().format("%d-%m %Hh%M").to_string();
    let to = copy_path(Path::new(path), &when, |p| p.exists()).ok_or("não achei a pasta da LegatIA")?;
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&to, bytes).map_err(|e| e.to_string())?;
    Ok(to)
}

/// O clique em "Atualizar": guarda a minha versão (se mudei algo), pede a versão da nuvem ao programa e, quando ele
/// responder, relê o arquivo na aba (ver [`frame`]).
#[cfg(not(target_arch = "wasm32"))]
pub fn start(app: &mut FigApp, ctx: &egui::Context) {
    if state().running {
        return;
    }
    let Some(ds) = app.session.docs.get(app.session.active) else { return };
    let (Some(path), dirty) = (ds.path.clone(), ds.is_dirty()) else { return };
    let mine = if dirty {
        match keep_mine(app, &path) {
            Ok(p) => Some(p),
            Err(e) => {
                app.toast(format!("Não atualizei: {e}. Use “Salvar como” para guardar a sua versão antes."));
                return;
            }
        }
    } else {
        None
    };
    state().running = true;
    let ctx = ctx.clone();
    let _ = std::thread::Builder::new().name("legatia-update".into()).spawn(move || {
        let result = match super::sync::post(&format!("/native/update?path={}", super::sync::encode_query(&path)), 120) {
            Some((200, body)) => Ok(serde_json::from_str::<serde_json::Value>(&body).ok().and_then(|v| v.get("copy").and_then(|c| c.as_str()).map(str::to_string))),
            Some((_, body)) => Err(serde_json::from_str::<serde_json::Value>(&body).ok().and_then(|v| v.get("message").and_then(|m| m.as_str()).map(str::to_string)).unwrap_or_else(|| "o programa da LegatIA recusou.".into())),
            None => Err("o programa da LegatIA não respondeu.".into()),
        };
        let mut s = state();
        s.running = false;
        s.done = Some(Done { path, result, mine });
        drop(s);
        ctx.request_repaint();
    });
}

/// A resposta do programa, no quadro: relê o arquivo na aba.
#[cfg(not(target_arch = "wasm32"))]
pub fn frame(app: &mut FigApp) {
    let done = state().done.take();
    let Some(done) = done else { return };
    let Some(index) = app.session.docs.iter().position(|d| d.path.as_deref() == Some(done.path.as_str())) else { return };
    // Sem o programa, o arquivo que mudou no disco ainda pode ser relido (a minha versão já está guardada)
    let changed = app.session.docs.get(index).is_some_and(|d| d.changed_on_disk());
    if let Err(why) = &done.result
        && !changed
    {
        app.toast(format!("Não deu para atualizar: {why}"));
        return;
    }
    app.session.active = index;
    if app.run("file.revert", serde_json::json!({})).is_none() {
        return;
    }
    state().looked = None;
    let kept = done.mine.is_some() || matches!(done.result, Ok(Some(_)));
    app.toast(if kept { "Arquivo atualizado. A sua versão ficou guardada na pasta _conflitos." } else { "Arquivo atualizado." });
}

/// Salvar uma aba da pasta da LegatIA cujo arquivo mudou no disco: avisa o que fazer em vez da recusa em inglês. `true`: não salve.
#[cfg(not(target_arch = "wasm32"))]
pub fn blocks_save(app: &mut FigApp) -> bool {
    let Some(path) = app.session.docs.get(app.session.active).and_then(|d| d.path.clone()) else { return false };
    if !super::sync::in_folder(&path) || !app.session.docs.get(app.session.active).is_some_and(|d| d.changed_on_disk()) {
        return false;
    }
    app.toast("Há uma versão mais nova deste arquivo. Clique em Atualizar: o que você mudou fica guardado em _conflitos.");
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_copy_goes_to_conflicts_and_never_over_another_copy() {
        let path = Path::new("raiz").join("Loja").join("Home.figcraft");
        let first = copy_path(&path, "26-11 10h00", |_| false).unwrap();
        assert_eq!(first, Path::new("raiz").join("_conflitos").join("Loja - Home (minha cópia 26-11 10h00).figcraft"));
        let second = copy_path(&path, "26-11 10h00", |p| p == first).unwrap();
        assert_eq!(second, Path::new("raiz").join("_conflitos").join("Loja - Home (minha cópia 26-11 10h00) (2).figcraft"));
    }
}
