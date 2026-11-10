//! LegatIA: o orbe que chama uma IA deste computador para mexer no editor, pelo MCP.
//!
//! O orbe fica no fim da barra de ferramentas (o espaço que o `host` reserva, [`super::host::dock_slot_rect`]).
//! Um clique abre o painel: a pessoa escolhe qual IA instalada vai receber o pedido e escreve o que quer.
//!
//! - **Executa sozinha** (Claude Code, Codex): o pedido vai para a IA por um comando sem janela, com a ponte MCP
//!   (`figcraft-cli mcp --attach`) já indicada na própria chamada, sem depender de configuração prévia. A IA trabalha no
//!   editor aberto e a resposta aparece no painel.
//! - **Abre e cola** (Claude Desktop, Antigravity, Windsurf, Cursor, VS Code): não há jeito seguro de escrever no chat
//!   delas. O pedido é copiado, o programa abre e a pessoa cola e dá Enter. Elas precisam ter sido ligadas ao editor
//!   (`Conectar-IA.cmd`).
//!
//! Nada sai deste computador além do que a própria IA escolhida já envia ao serviço dela. O texto do pedido segue pela
//! entrada padrão do processo, nunca por uma linha de comando.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use egui::{Align2, Color32, CornerRadius, Id, Order, Rect, Sense, Stroke, Ui, pos2, vec2};

use super::strands::{self, Anim, Params};

use crate::FigApp;
use crate::theme::{Tokens, regular, semibold};

/// A porta do canal local do app: a do `--control` com que ele abriu (os atalhos da instalação usam 47800).
static PORT: AtomicU16 = AtomicU16::new(47800);

/// O app diz em que porta abriu o canal local (`--control`).
pub fn set_port(port: u16) {
    PORT.store(port, Ordering::Relaxed);
}

fn port() -> u16 {
    PORT.load(Ordering::Relaxed)
}
const SERVER: &str = "legatia-design";
/// Quanto a IA pode levar antes de o painel desistir.
const LIMIT: Duration = Duration::from_secs(15 * 60);

/// Os padrões de qualidade que acompanham todo pedido (um guia maior entra aqui depois).
const GUIDE: &str = "Padrões da LegatIA: use auto layout (layout, gap, padding, sizingH/sizingV hug ou fill) em vez de posições soltas; \
use as variáveis e estilos do arquivo (cores, tipografia) antes de valores crus; a fonte da marca é Satoshi e o azul é #006FFE; \
dê nomes claros às camadas.";

#[derive(Clone, Debug)]
enum How {
    /// Claude Code: `claude -p` com o pedido na entrada padrão.
    Claude(PathBuf),
    /// Codex: `codex exec -` com o pedido na entrada padrão.
    Codex(PathBuf),
    /// Programa de janela: copia o pedido e abre.
    Open(Launch),
}

#[derive(Clone, Debug)]
enum Launch {
    Exe(PathBuf),
    /// Um app da Microsoft Store, pelo identificador `shell:AppsFolder\...`.
    Store(String),
}

#[derive(Clone, Debug)]
struct Ai {
    name: &'static str,
    how: How,
    /// O arquivo de configuração de MCP dela: sem a entrada `legatia-design` nele, ela não enxerga o editor.
    config: Option<PathBuf>,
}

impl Ai {
    fn runs_alone(&self) -> bool {
        !matches!(self.how, How::Open(_))
    }

    /// Pronta para receber um pedido: as que executam sozinhas levam a ponte junto, as outras precisam da entrada na configuração.
    fn connected(&self) -> bool {
        self.runs_alone() || self.config.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).is_some_and(|t| t.contains(SERVER))
    }
}

struct Run {
    ai: &'static str,
    cancel: Arc<AtomicBool>,
    done: bool,
    ok: bool,
    output: String,
}

struct State {
    open: bool,
    ais: Option<Vec<Ai>>,
    selected: usize,
    text: String,
    run: Option<Run>,
    notice: Option<String>,
    focus: bool,
    /// A seleção do editor vai no pedido ("Tirar" desliga).
    use_selection: bool,
}

static STATE: Mutex<State> = Mutex::new(State { open: false, ais: None, selected: 0, text: String::new(), run: None, notice: None, focus: false, use_selection: true });

fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

// ---------------------------------------------------------------------------------------------- achar as IAs

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from)
}

fn on_path(file: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(file)).find(|p| p.is_file())
}

/// O `claude.exe` de verdade: no PATH, na pasta do pacote npm (o `claude.cmd` é só um atalho para ele) ou no instalador próprio.
fn find_claude() -> Option<PathBuf> {
    if let Some(p) = on_path("claude.exe") {
        return Some(p);
    }
    if let Some(shim) = on_path("claude.cmd") {
        let real = shim.parent()?.join("node_modules").join("@anthropic-ai").join("claude-code").join("bin").join("claude.exe");
        if real.is_file() {
            return Some(real);
        }
    }
    let own = env_path("USERPROFILE")?.join(".local").join("bin").join("claude.exe");
    own.is_file().then_some(own)
}

/// O `codex.exe` de verdade, dentro do pacote npm (`vendor\<alvo>in\codex.exe`).
fn find_codex() -> Option<PathBuf> {
    if let Some(p) = on_path("codex.exe") {
        return Some(p);
    }
    let shim = on_path("codex.cmd")?;
    let target = shim.parent()?.join("node_modules").join("@openai").join("codex").join("node_modules").join("@openai");
    for platform in std::fs::read_dir(&target).ok()?.flatten() {
        let vendor = platform.path().join("vendor");
        for triple in std::fs::read_dir(&vendor).into_iter().flatten().flatten() {
            // O pacote guarda o programa em `bin\codex.exe` (versões antigas: `codex\codex.exe`).
            for exe in [triple.path().join("bin").join("codex.exe"), triple.path().join("codex").join("codex.exe")] {
                if exe.is_file() {
                    return Some(exe);
                }
            }
        }
    }
    None
}

fn store_family(prefix: &str) -> Option<String> {
    let packages = env_path("LOCALAPPDATA")?.join("Packages");
    std::fs::read_dir(packages).ok()?.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).find(|n| n.starts_with(prefix))
}

fn store_app(prefix: &str) -> Option<String> {
    Some(format!("shell:AppsFolder\\{}!Claude", store_family(prefix)?))
}

/// A configuração do Claude Desktop: a do pacote da Microsoft Store quando ele é esse, senão a de `%APPDATA%\\Claude`.
fn claude_desktop_config(roaming: &Path, local: &Path) -> PathBuf {
    let plain = roaming.join("Claude").join("claude_desktop_config.json");
    let Some(family) = store_family("Claude_") else { return plain };
    let packaged = local.join("Packages").join(family).join("LocalCache").join("Roaming").join("Claude").join("claude_desktop_config.json");
    if packaged.parent().is_some_and(Path::is_dir) { packaged } else { plain }
}

fn detect() -> Vec<Ai> {
    let local = env_path("LOCALAPPDATA").unwrap_or_default();
    let roaming = env_path("APPDATA").unwrap_or_default();
    let home = env_path("USERPROFILE").unwrap_or_default();
    let mut out = Vec::new();
    if let Some(exe) = find_claude() {
        out.push(Ai { name: "Claude Code", how: How::Claude(exe), config: None });
    }
    if let Some(exe) = find_codex() {
        out.push(Ai { name: "Codex", how: How::Codex(exe), config: None });
    }
    if let Some(app) = store_app("Claude_") {
        out.push(Ai { name: "Claude Desktop", how: How::Open(Launch::Store(app)), config: Some(claude_desktop_config(&roaming, &local)) });
    } else if local.join("AnthropicClaude").is_dir() {
        let exe = local.join("AnthropicClaude").join("claude.exe");
        if exe.is_file() {
            out.push(Ai { name: "Claude Desktop", how: How::Open(Launch::Exe(exe)), config: Some(roaming.join("Claude").join("claude_desktop_config.json")) });
        }
    }
    let programs = local.join("Programs");
    let windows: [(&'static str, PathBuf, PathBuf); 4] = [
        ("Antigravity", programs.join("Antigravity").join("Antigravity.exe"), home.join(".gemini").join("antigravity").join("mcp_config.json")),
        ("Windsurf", programs.join("Windsurf").join("Windsurf.exe"), home.join(".codeium").join("windsurf").join("mcp_config.json")),
        ("Cursor", programs.join("cursor").join("Cursor.exe"), home.join(".cursor").join("mcp.json")),
        ("VS Code", programs.join("Microsoft VS Code").join("Code.exe"), roaming.join("Code").join("User").join("mcp.json")),
    ];
    for (name, exe, config) in windows {
        if exe.is_file() {
            out.push(Ai { name, how: How::Open(Launch::Exe(exe)), config: Some(config) });
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------- o pedido

/// O texto que vai para a IA: o contexto do editor, os padrões e o pedido da pessoa.
fn prompt(request: &str, file: Option<&str>, layers: &[(u64, String)]) -> String {
    let mut text = format!("Use o conector \"{SERVER}\" (MCP) para trabalhar no editor LegatIA Design que está aberto neste computador.");
    if let Some(file) = file {
        text.push_str(&format!(" O arquivo aberto é \"{file}\"."));
    }
    if !layers.is_empty() {
        let shown: Vec<String> = layers.iter().take(8).map(|(id, name)| format!("\"{name}\" (id {id})")).collect();
        text.push_str(&format!(" Trabalhe na seleção: {}{}.", shown.join(", "), if layers.len() > 8 { format!(" e mais {}", layers.len() - 8) } else { String::new() }));
    }
    text.push_str(" Confira o resultado com um print (screenshot) antes de terminar e me diga o que mudou. ");
    text.push_str(GUIDE);
    text.push_str("\n\nPedido: ");
    text.push_str(request.trim());
    text
}

fn context(app: &FigApp) -> (Option<String>, Vec<(u64, String)>) {
    let file = app
        .session
        .docs
        .get(app.session.active)
        .and_then(|d| d.path.as_deref().and_then(|p| Path::new(p).file_stem().map(|s| s.to_string_lossy().into_owned())))
        .or_else(|| app.session.doc().ok().map(|_| "sem título".to_string()));
    let layers = match app.session.doc() {
        Ok(doc) => app.session.selection().into_iter().filter_map(|id| doc.node(id).map(|n| (id.0, n.name.clone()))).collect(),
        Err(_) => Vec::new(),
    };
    (file, layers)
}

// ---------------------------------------------------------------------------------------------- executar

/// O canal local do app está aberto? (Sem ele a ponte MCP não tem com quem falar.)
fn channel_open() -> bool {
    std::net::TcpStream::connect_timeout(&std::net::SocketAddr::from(([127, 0, 0, 1], port())), Duration::from_millis(400)).is_ok()
}

fn cli_path() -> Option<PathBuf> {
    let cli = std::env::current_exe().ok()?.parent()?.join("figcraft-cli.exe");
    cli.is_file().then_some(cli)
}

/// Um valor de texto TOML (aspas duplas, barra invertida e aspas escapadas).
fn toml_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn base_command(exe: &Path) -> Command {
    let mut c = Command::new(exe);
    // Sem janela de console: o app é um programa de janela e a IA também roda escondida.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    if let Some(home) = env_path("USERPROFILE") {
        let folder = home.join("LegatIA Design");
        c.current_dir(if folder.is_dir() { folder } else { home });
    }
    c
}

/// A configuração de MCP que o Claude Code lê da própria chamada (um arquivo temporário; o pedido nunca passa por ele).
fn claude_config(cli: &Path) -> Option<PathBuf> {
    let json = serde_json::json!({"mcpServers": {SERVER: {"command": cli, "args": ["mcp", "--attach", port().to_string()]}}});
    let path = std::env::temp_dir().join("legatia-design-mcp.json");
    std::fs::write(&path, serde_json::to_vec(&json).ok()?).ok()?;
    Some(path)
}

fn spawn_run(ctx: egui::Context, ai: Ai, text: String) {
    let cancel = Arc::new(AtomicBool::new(false));
    state().run = Some(Run { ai: ai.name, cancel: cancel.clone(), done: false, ok: false, output: String::new() });
    let finish = move |ok: bool, output: String| {
        if let Some(run) = state().run.as_mut() {
            run.done = true;
            run.ok = ok;
            run.output = output;
        }
        ctx.request_repaint();
    };
    let _ = std::thread::Builder::new().name("legatia-agent".into()).spawn(move || {
        let Some(cli) = cli_path() else {
            return finish(false, "Não achei o figcraft-cli.exe ao lado do app. Reinstale o LegatIA Design.".into());
        };
        if !channel_open() {
            return finish(false, format!("O canal local do app (porta {}) está fechado, então a IA não tem como mexer no editor. Abra o LegatIA Design pelo atalho do menu Iniciar.", port()));
        }
        let mut command = match &ai.how {
            How::Claude(exe) => {
                let Some(config) = claude_config(&cli) else { return finish(false, "Não consegui preparar a conexão com o editor.".into()) };
                let mut c = base_command(exe);
                c.arg("-p").arg("--mcp-config").arg(config).arg("--allowedTools").arg(format!("mcp__{SERVER}"));
                c
            }
            How::Codex(exe) => {
                let mut c = base_command(exe);
                c.args(["exec", "--skip-git-repo-check", "-c"])
                    .arg(format!("mcp_servers.{SERVER}.command={}", toml_string(&cli.to_string_lossy())))
                    .arg("-c")
                    .arg(format!("mcp_servers.{SERVER}.args=[\"mcp\", \"--attach\", \"{}\"]", port()))
                    .arg("-");
                c
            }
            How::Open(_) => return finish(false, String::new()),
        };
        command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = match command.spawn() {
            Ok(c) => c,
            Err(e) => return finish(false, format!("Não consegui abrir {}: {e}", ai.name)),
        };
        // O pedido vai pela entrada padrão e a entrada fecha: a IA lê até o fim.
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let read = |mut pipe: Box<dyn Read + Send>| {
            std::thread::spawn(move || {
                let mut buf = Vec::new();
                let _ = pipe.read_to_end(&mut buf);
                String::from_utf8_lossy(&buf).into_owned()
            })
        };
        let out = child.stdout.take().map(|p| read(Box::new(p)));
        let err = child.stderr.take().map(|p| read(Box::new(p)));
        let started = Instant::now();
        let status = loop {
            if cancel.load(Ordering::Relaxed) {
                let _ = child.kill();
                let _ = child.wait();
                return finish(false, "Parado.".into());
            }
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if started.elapsed() > LIMIT => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return finish(false, "A IA passou de 15 minutos e foi parada.".into());
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(150)),
                Err(e) => return finish(false, format!("Falha ao acompanhar a IA: {e}")),
            }
        };
        let stdout = out.and_then(|h| h.join().ok()).unwrap_or_default();
        let stderr = err.and_then(|h| h.join().ok()).unwrap_or_default();
        let shown = if stdout.trim().is_empty() { stderr } else { stdout };
        finish(status.success(), shown.trim().to_string());
    });
}

// ---------------------------------------------------------------------------------------------- ligar a IA ao editor

/// As duas entradas da LegatIA, como texto JSON (sem a vírgula do fim): ao vivo (`--attach`) e sem abrir o app.
fn entries_json(cli: &Path, port: u16, typed: bool) -> String {
    let cli = serde_json::to_string(&cli.to_string_lossy()).unwrap_or_else(|_| "\"\"".into());
    let kind = if typed { "\"type\": \"stdio\", " } else { "" };
    format!(
        "\"{SERVER}\": {{ {kind}\"command\": {cli}, \"args\": [\"mcp\", \"--attach\", \"{port}\"] }},\n    \"{SERVER}-offline\": {{ {kind}\"command\": {cli}, \"args\": [\"mcp\"] }}"
    )
}

/// Acrescenta as entradas da LegatIA ao bloco `key` do arquivo JSON de configuração de MCP de uma IA (`mcpServers`; `servers` no VS Code),
/// sem mexer no resto do texto, com uma cópia do arquivo antigo ao lado (`.bak-legatia`, só na primeira vez). Arquivo que não existe é criado;
/// um que não é JSON puro (comentários, por exemplo) fica como está e a pessoa recebe o aviso. Já tendo as entradas, não faz nada.
fn add_entries(path: &Path, cli: &Path, port: u16, key: &str, typed: bool) -> Result<(), String> {
    let existing = std::fs::read_to_string(path).ok().filter(|t| !t.trim().is_empty());
    let entries = entries_json(cli, port, typed);
    let new_text = match &existing {
        None => format!("{{\n  \"{key}\": {{\n    {entries}\n  }}\n}}\n"),
        Some(text) => {
            if text.contains(SERVER) {
                return Ok(());
            }
            let original: serde_json::Value = serde_json::from_str(text).map_err(|_| format!("O arquivo {} não é um JSON simples (tem comentários?), então não mexi nele. Rode Conectar-IA.cmd ou acrescente as entradas à mão.", path.display()))?;
            let marker = format!("\"{key}\"");
            let opened = text.find(&marker).and_then(|at| {
                let after = &text[at + marker.len()..];
                let colon = after.trim_start().strip_prefix(':')?;
                let brace = colon.trim_start().strip_prefix('{')?;
                Some(text.len() - brace.len())
            });
            match opened {
                // Entra logo depois da chave de abertura do bloco, e o resto do arquivo segue igual
                Some(at) => {
                    let rest = &text[at..];
                    let comma = if rest.trim_start().starts_with('}') { "" } else { "," };
                    format!("{}\n    {entries}{comma}{rest}", &text[..at])
                }
                // O arquivo não tem o bloco: entra como chave nova
                None => {
                    let mut value = original;
                    let object = value.as_object_mut().ok_or_else(|| format!("O arquivo {} não é um objeto JSON.", path.display()))?;
                    let block: serde_json::Value = serde_json::from_str(&format!("{{ {entries} }}")).map_err(|e| e.to_string())?;
                    object.insert(key.to_string(), block);
                    serde_json::to_string_pretty(&value).map_err(|e| e.to_string())? + "\n"
                }
            }
        }
    };
    // Só grava um arquivo que ainda é JSON e que traz as duas entradas
    let check: serde_json::Value = serde_json::from_str(&new_text).map_err(|e| format!("Não consegui acrescentar as entradas sem estragar {}: {e}", path.display()))?;
    if check.get(key).and_then(|b| b.get(SERVER)).is_none() {
        return Err(format!("Não consegui acrescentar as entradas a {}.", path.display()));
    }
    if existing.is_some() {
        let backup = PathBuf::from(format!("{}.bak-legatia", path.display()));
        if !backup.exists() {
            std::fs::copy(path, &backup).map_err(|e| format!("Não consegui guardar a cópia de {}: {e}", path.display()))?;
        }
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Não consegui criar {}: {e}", dir.display()))?;
    }
    std::fs::write(path, new_text).map_err(|e| format!("Não consegui gravar {}: {e}", path.display()))
}

/// Liga a IA de janela ao editor, na primeira vez que ela recebe um pedido. O que foi feito, para o painel dizer.
fn connect(ai: &Ai) -> Result<String, String> {
    let config = ai.config.as_ref().ok_or_else(|| format!("Não sei onde o {} guarda a configuração de MCP.", ai.name))?;
    let cli = cli_path().ok_or("Não achei o figcraft-cli.exe ao lado do app. Reinstale o LegatIA Design.")?;
    let (key, typed) = if ai.name == "VS Code" { ("servers", true) } else { ("mcpServers", false) };
    add_entries(config, &cli, port(), key, typed)?;
    let file = config.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    Ok(format!("Liguei o {} ao editor (a configuração antiga ficou guardada em {file}.bak-legatia).", ai.name))
}

/// Copia o pedido e abre o programa; ele não executa sozinho.
fn open_and_copy(ctx: &egui::Context, ai: &Ai, text: &str) {
    ctx.copy_text(text.to_string());
    let How::Open(launch) = &ai.how else { return };
    let spawned = match launch {
        Launch::Exe(exe) => Command::new(exe).spawn().map(|_| ()),
        Launch::Store(app) => Command::new("explorer.exe").arg(app).spawn().map(|_| ()),
    };
    let said = match spawned {
        Ok(()) => format!("Pedido copiado. Abri o {}: cole no chat (Ctrl+V) e dê Enter.", ai.name),
        Err(e) => format!("O pedido foi copiado, mas não consegui abrir o {}: {e}", ai.name),
    };
    let mut s = state();
    // Se acabou de ligar a IA ao editor, a primeira frase conta isso
    s.notice = Some(match s.notice.take() {
        Some(connected) => format!("{connected} {said} Se ele já estava aberto, feche e abra de novo para ver o editor."),
        None => said,
    });
}

// ---------------------------------------------------------------------------------------------- desenho

/// Quanto o orbe cresce com o painel aberto (o `GROW` do site).
const GROW: f32 = 1.9;
/// Sem mexer no mouse nem no teclado por esse tempo (ou com a janela em outro app), o orbe para onde está.
const IDLE_SECS: f64 = 20.0;
const BLUE: Color32 = Color32::from_rgb(0x00, 0x6F, 0xFE);

/// O movimento do orbe entre um quadro e outro (o que o site guarda dentro do `Strands`).
struct OrbState {
    anim: Anim,
    texture: Option<egui::TextureHandle>,
    last_step: f64,
    last_draw: f64,
    last_input: f64,
    /// O pulso de 0 a 1 de cada ação do agente; volta a acalmar sozinho.
    pulse: f32,
    last_call: Option<Instant>,
}

static ORB: Mutex<Option<OrbState>> = Mutex::new(None);

/// O orbe no espaço reservado do dock (cresce para cima ao abrir) e, aberto, o painel acima dele.
pub fn orb(app: &mut FigApp, ui: &mut Ui, t: &Tokens) {
    let ctx = ui.ctx().clone();
    let Some(slot) = super::host::dock_slot_rect(&ctx) else { return };
    let now = ctx.input(|i| i.time);
    let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
    let moved = ctx.input(|i| {
        i.pointer.is_moving() || i.pointer.any_pressed() || i.events.iter().any(|e| matches!(e, egui::Event::Key { .. } | egui::Event::Text(_) | egui::Event::MouseWheel { .. }))
    });

    // Ctrl+L abre o painel já com a seleção (como no editor do site).
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::L)) {
        let mut s = state();
        s.open = true;
        s.focus = true;
        s.use_selection = true;
        if s.ais.is_none() {
            s.ais = Some(detect());
        }
    }

    let presence = app.session.agent_presence_at(Instant::now()).map(|p| p.at);
    let running = state().run.as_ref().is_some_and(|r| !r.done);
    let working = presence.is_some() || running;
    let open = state().open;

    let grow = ctx.animate_value_with_time(Id::unique("legatia_orb_grow"), if open { GROW } else { 1.0 }, 0.3);
    let base = slot.width();
    let size = base * grow;
    let top = slot.max.y - size + 6.0 * (grow - 1.0) / (GROW - 1.0);
    let at = pos2(slot.center().x - size / 2.0, top);

    let mut orb_state = ORB.lock().unwrap_or_else(|e| e.into_inner());
    let o = orb_state.get_or_insert_with(|| OrbState { anim: Anim::default(), texture: None, last_step: now, last_draw: 0.0, last_input: now, pulse: 0.0, last_call: None });
    if moved {
        o.last_input = now;
    }
    // Cada ferramenta que o agente chama pulsa o orbe.
    if presence.is_some() && presence != o.last_call {
        o.pulse = 1.0;
    }
    o.last_call = presence;

    // O orbe é desenhado numa camada própria, por cima da dock (a textura é a do quadro anterior).
    let texture_id = o.texture.as_ref().map(egui::TextureHandle::id);
    let (r, resp) = egui::Area::new(Id::unique("legatia_orb_area"))
        .order(Order::Foreground)
        .fixed_pos(at)
        .show(&ctx, |ui| {
            let (r, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
            crate::a11y::button(&resp, "Falar com a IA");
            if let Some(id) = texture_id {
                ui.painter().image(id, r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            (r, resp)
        })
        .inner;

    let clicked = resp.clicked();
    let hovering = resp.hovered();
    let awake = focused && (now - o.last_input < IDLE_SECS || working || open || hovering);
    o.anim.pointer(resp.hover_pos().map(|p| ((p.x - r.center().x) / (size / 2.0), (p.y - r.center().y) / (size / 2.0))));
    if clicked {
        o.anim.click();
    }
    let fps = if working || open { 30.0 } else { 15.0 };
    let due = o.texture.is_none() || (awake && now - o.last_draw >= 1.0 / fps - 0.003);
    if due {
        let dt = (now - o.last_step) as f32;
        o.pulse *= 0.92f32.powf(dt.clamp(0.0, 0.1) * 60.0);
        if o.pulse < 0.03 {
            o.pulse = 0.0;
        }
        let frame = o.anim.step(dt, Params { working, open, level: (o.pulse > 0.0).then_some(o.pulse) });
        let px = ((size * ctx.pixels_per_point()).round() as usize).clamp(24, 160);
        let bytes = strands::render(px, &frame);
        let image = egui::ColorImage::from_rgba_premultiplied([px, px], &bytes);
        match o.texture.as_mut() {
            Some(tex) => tex.set(image, egui::TextureOptions::LINEAR),
            None => o.texture = Some(ctx.load_texture("legatia_orb", image, egui::TextureOptions::LINEAR)),
        }
        o.last_draw = now;
        o.last_step = now;
    }
    if awake {
        ctx.request_repaint_after(Duration::from_secs_f64(1.0 / fps));
    } else {
        // Dormindo: nenhum quadro; ao acordar o tempo parado não vira um salto.
        o.last_step = now;
    }
    drop(orb_state);
    let _ = resp.on_hover_text(if working { "A IA está trabalhando aqui" } else { "Pedir à IA (Ctrl+L)" });
    if clicked {
        let mut s = state();
        s.open = !s.open;
        s.focus = s.open;
        s.use_selection = true;
        if s.ais.is_none() {
            s.ais = Some(detect());
        }
    }
    if state().open {
        panel(app, &ctx, t, r, working);
    }
}

fn describe(layers: &[(u64, String)]) -> String {
    let shown: Vec<String> = layers.iter().take(3).map(|(_, n)| format!("\"{n}\"")).collect();
    if layers.len() > shown.len() { format!("{} e mais {}", shown.join(", "), layers.len() - shown.len()) } else { shown.join(", ") }
}

fn panel(app: &mut FigApp, ctx: &egui::Context, t: &Tokens, orb: Rect, working: bool) {
    let (file, layers) = context(app);
    let mut close = false;
    let mut send: Option<(Ai, String)> = None;
    let mut stop = false;
    let screen = ctx.content_rect();
    let left = (orb.center().x - 190.0).clamp(12.0, (screen.max.x - 384.0).max(12.0));
    let area = egui::Area::new(Id::unique("legatia_orb_panel"))
        .order(Order::Foreground)
        .pivot(Align2::LEFT_BOTTOM)
        .fixed_pos(pos2(left, orb.min.y - 10.0))
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(Color32::from_rgba_unmultiplied(16, 18, 24, 228))
                .stroke(Stroke::new(1.0, Color32::from_white_alpha(26)))
                .corner_radius(CornerRadius::same(22))
                .shadow(egui::Shadow { offset: [0, 10], blur: 34, spread: 0, color: Color32::BLACK.gamma_multiply(0.5) })
                .inner_margin(egui::Margin::same(16))
                .show(ui, |ui| {
                    ui.set_width(340.0);
                    let mut s = state();
                    ui.horizontal(|ui| {
                        let title = if working { "A IA está trabalhando no editor." } else { "O que a IA faz aqui?" };
                        ui.label(egui::RichText::new(title).font(regular(13.0)).color(t.text));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add(egui::Button::new(egui::RichText::new("×").font(regular(16.0)).color(t.text_secondary)).frame(false)).clicked() {
                                close = true;
                            }
                        });
                    });
                    ui.add_space(10.0);

                    let ais = s.ais.clone().unwrap_or_default();
                    if ais.is_empty() {
                        ui.label(egui::RichText::new("Não achei nenhuma IA neste computador. Instale o Claude, o Codex ou outra IA com MCP e rode Conectar-IA.cmd, na pasta do app.").font(regular(11.5)).color(t.text_secondary));
                        return;
                    }
                    if s.selected >= ais.len() {
                        s.selected = 0;
                    }
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                        for (i, ai) in ais.iter().enumerate() {
                            let on = s.selected == i;
                            let label = egui::RichText::new(ai.name).font(regular(11.5)).color(if on { Color32::WHITE } else { t.text });
                            let b = egui::Button::new(label).corner_radius(CornerRadius::same(12)).fill(if on { BLUE } else { t.field }).stroke(Stroke::new(1.0, if on { BLUE } else { t.border }));
                            if ui.add(b).clicked() {
                                s.selected = i;
                            }
                        }
                    });
                    let ai = ais[s.selected].clone();
                    let hint = if !ai.connected() {
                        format!("O {} ainda não está ligado ao editor: o LegatIA Design liga agora, no primeiro envio.", ai.name)
                    } else if ai.runs_alone() {
                        format!("O {} trabalha sozinho no editor aberto e a resposta aparece aqui.", ai.name)
                    } else {
                        format!("Copio o pedido e abro o {}: é só colar (Ctrl+V) e dar Enter.", ai.name)
                    };
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new(hint).font(regular(11.0)).color(t.text_secondary));
                    ui.add_space(8.0);

                    // A seleção do editor vai no pedido; "Tirar" deixa só o texto.
                    if s.use_selection && !layers.is_empty() {
                        egui::Frame::NONE.fill(t.field).stroke(Stroke::new(1.0, t.border)).corner_radius(CornerRadius::same(12)).inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(format!("Seleção: {}", describe(&layers))).font(regular(11.0)).color(t.text_secondary));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.add(egui::Button::new(egui::RichText::new("Tirar").font(regular(11.0)).color(t.text)).frame(false)).clicked() {
                                        s.use_selection = false;
                                    }
                                });
                            });
                        });
                        ui.add_space(6.0);
                    } else if let Some(file) = &file {
                        ui.label(egui::RichText::new(format!("Arquivo: {file}")).font(regular(11.0)).color(t.text_secondary));
                        ui.add_space(6.0);
                    }

                    let busy = s.run.as_ref().is_some_and(|r| !r.done);
                    let hint_text = if s.use_selection && !layers.is_empty() {
                        "Ex.: deixa esses textos no padrão LegatIA e alinha o espaçamento"
                    } else {
                        "Ex.: cria um card de planos no estilo LegatIA ao lado do quadro selecionado"
                    };
                    let text_id = Id::unique("legatia_orb_text");
                    let submit_key = ui.memory(|m| m.has_focus(text_id)) && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter));
                    let edit = egui::TextEdit::multiline(&mut s.text).id(text_id).hint_text(hint_text).font(regular(12.5)).desired_rows(3).desired_width(f32::INFINITY).interactive(!busy);
                    let out = ui.add(edit);
                    if s.focus {
                        out.request_focus();
                        s.focus = false;
                    }
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if busy {
                            ui.label(egui::RichText::new(format!("{} trabalhando…", s.run.as_ref().map_or("", |r| r.ai))).font(regular(11.5)).color(t.text_secondary));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.add(egui::Button::new(egui::RichText::new("Parar").font(semibold(11.5)).color(t.text)).corner_radius(CornerRadius::same(14))).clicked() {
                                    stop = true;
                                }
                            });
                        } else {
                            ui.label(egui::RichText::new("Ctrl+Enter envia").font(regular(10.5)).color(t.icon_secondary));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let ready = !s.text.trim().is_empty();
                                let b = egui::Button::new(egui::RichText::new("Enviar").font(semibold(11.5)).color(Color32::WHITE))
                                    .corner_radius(CornerRadius::same(14))
                                    .fill(if ready { BLUE } else { t.hover })
                                    .stroke(Stroke::NONE)
                                    .min_size(vec2(76.0, 28.0));
                                if (ui.add_enabled(ready, b).clicked() || (submit_key && ready)) && !s.text.trim().is_empty() {
                                    let chosen: &[(u64, String)] = if s.use_selection { &layers } else { &[] };
                                    send = Some((ai.clone(), prompt(&s.text, file.as_deref(), chosen)));
                                }
                            });
                        }
                    });
                    if let Some(note) = s.notice.clone() {
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new(note).font(regular(11.0)).color(t.text_secondary));
                    }
                    if let Some(run) = s.run.as_ref().filter(|r| r.done) {
                        ui.add_space(8.0);
                        let (title, color) = if run.ok { (format!("{} terminou", run.ai), t.success) } else { (format!("{} não concluiu", run.ai), t.danger) };
                        ui.label(egui::RichText::new(title).font(semibold(11.5)).color(color));
                        if !run.output.is_empty() {
                            egui::ScrollArea::vertical().max_height(170.0).auto_shrink([false, true]).show(ui, |ui| {
                                ui.add(egui::Label::new(egui::RichText::new(&run.output).font(regular(11.5)).color(t.text)).selectable(true));
                            });
                        }
                    }
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new("Vai para a IA que você escolher, com o seu plano. Ctrl+L no editor abre com a seleção.").font(regular(10.0)).color(t.icon_secondary));
                });
        });
    // Esc e um clique fora do painel e do orbe fecham.
    let panel_rect = area.response.rect;
    let outside = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !panel_rect.contains(p) && !orb.contains(p)));
    if close || outside || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        state().open = false;
    }
    if stop {
        if let Some(run) = state().run.as_ref() {
            run.cancel.store(true, Ordering::Relaxed);
        }
    }
    if let Some((ai, text)) = send {
        {
            let mut s = state();
            s.notice = None;
            s.run = None;
        }
        if ai.runs_alone() {
            spawn_run(ctx.clone(), ai, text);
        } else {
            // A primeira vez: liga a IA ao editor (as entradas da LegatIA na configuração dela) antes de abrir
            if !ai.connected() {
                match connect(&ai) {
                    Ok(done) => state().notice = Some(done),
                    Err(e) => {
                        ctx.copy_text(text);
                        state().notice = Some(format!("{e} O pedido ficou copiado."));
                        return;
                    }
                }
            }
            if !channel_open() {
                state().notice = Some(format!("O canal local do app (porta {}) está fechado, então o {} não alcança o editor. Abra o LegatIA Design pelo atalho do menu Iniciar.", port(), ai.name));
            }
            open_and_copy(ctx, &ai, &text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_names_the_connector_the_file_and_the_selection() {
        let p = prompt("  deixe o título maior ", Some("Home"), &[(4, "Título".into()), (7, "Card".into())]);
        assert!(p.contains("legatia-design"));
        assert!(p.contains("\"Home\""));
        assert!(p.contains("\"Título\" (id 4)"));
        assert!(p.ends_with("Pedido: deixe o título maior"));
    }

    #[test]
    fn a_long_selection_is_cut_with_how_many_were_left_out() {
        let layers: Vec<(u64, String)> = (0..12).map(|i| (i, format!("c{i}"))).collect();
        assert!(prompt("x", None, &layers).contains("e mais 4"));
    }

    #[test]
    fn toml_strings_escape_backslashes_and_quotes() {
        assert_eq!(toml_string(r#"C:\a "b""#), r#""C:\\a \"b\"""#);
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("legatia-agent-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn servers(path: &Path, key: &str) -> Vec<String> {
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        v[key].as_object().unwrap().keys().cloned().collect()
    }

    #[test]
    fn connecting_adds_the_two_entries_and_leaves_the_rest_of_the_text_alone() {
        let dir = temp("add");
        let file = dir.join("mcp_config.json");
        let before = "{\n  \"mcpServers\": {\n    \"supabase\": { \"command\": \"npx\", \"args\": [\"-y\", \"x\"], \"env\": { \"TOKEN\": \"segredo\" } }\n  }\n}\n";
        std::fs::write(&file, before).unwrap();
        let cli = Path::new("C:\\Users\\x\\LegatIA Design\\figcraft-cli.exe");
        add_entries(&file, cli, 47800, "mcpServers", false).unwrap();
        let after = std::fs::read_to_string(&file).unwrap();
        assert_eq!(servers(&file, "mcpServers"), ["legatia-design", "legatia-design-offline", "supabase"]);
        // O que já estava continua igual, byte a byte (o trecho do supabase aparece inteiro)
        assert!(after.contains("\"supabase\": { \"command\": \"npx\", \"args\": [\"-y\", \"x\"], \"env\": { \"TOKEN\": \"segredo\" } }"));
        assert!(after.contains("figcraft-cli.exe") && after.contains("--attach") && after.contains("47800"));
        // A cópia do arquivo antigo é igual ao que havia
        assert_eq!(std::fs::read_to_string(dir.join("mcp_config.json.bak-legatia")).unwrap(), before);
        // Ligar de novo não duplica nada
        add_entries(&file, cli, 47800, "mcpServers", false).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), after);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn connecting_handles_an_empty_block_a_missing_block_and_a_new_file() {
        let dir = temp("shapes");
        let cli = Path::new("C:/x/figcraft-cli.exe");
        let empty = dir.join("empty.json");
        std::fs::write(&empty, "{ \"mcpServers\": {} }").unwrap();
        add_entries(&empty, cli, 47800, "mcpServers", false).unwrap();
        assert_eq!(servers(&empty, "mcpServers"), ["legatia-design", "legatia-design-offline"]);

        let other = dir.join("other.json");
        std::fs::write(&other, "{ \"theme\": \"dark\" }").unwrap();
        add_entries(&other, cli, 47800, "mcpServers", false).unwrap();
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&other).unwrap()).unwrap();
        assert_eq!(v["theme"], "dark");
        assert!(v["mcpServers"]["legatia-design"].is_object());

        let fresh = dir.join("sub").join("novo.json");
        add_entries(&fresh, cli, 47801, "servers", true).unwrap();
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&fresh).unwrap()).unwrap();
        assert_eq!(v["servers"]["legatia-design"]["type"], "stdio");
        assert_eq!(v["servers"]["legatia-design"]["args"][2], "47801");
        assert!(!dir.join("sub").join("novo.json.bak-legatia").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_file_that_is_not_plain_json_is_left_alone() {
        let dir = temp("jsonc");
        let file = dir.join("mcp.json");
        let text = "{\n  // comentário\n  \"servers\": {}\n}\n";
        std::fs::write(&file, text).unwrap();
        let error = add_entries(&file, Path::new("C:/x/figcraft-cli.exe"), 47800, "servers", true).unwrap_err();
        assert!(error.contains("não mexi"), "{error}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
        assert!(!dir.join("mcp.json.bak-legatia").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn programs_that_run_alone_need_no_configuration_the_others_do() {
        let alone = Ai { name: "x", how: How::Claude(PathBuf::from("c")), config: None };
        assert!(alone.runs_alone() && alone.connected());
        let window = Ai { name: "y", how: How::Open(Launch::Exe(PathBuf::from("c"))), config: Some(PathBuf::from("não existe.json")) };
        assert!(!window.runs_alone() && !window.connected());
    }
}
