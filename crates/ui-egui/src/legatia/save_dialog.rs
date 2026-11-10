//! LegatIA: o quadro "Salvar na LegatIA" de um arquivo que ainda não está na nuvem (um novo, ou um que está fora da pasta
//! `LegatIA Design`): a pessoa escolhe o **projeto** (a lista vem da conta dela, pelo programa de sincronização) e o
//! **nome**, e o app grava em `LegatIA Design\<projeto>\<nome>.figcraft` e manda enviar. Quem cria o arquivo na nuvem, no
//! projeto da pasta, é o próprio programa. Sem isso a pessoa teria de navegar até a pasta certa e acertar o nome do
//! projeto letra por letra. "Salvar só neste PC" abre o seletor comum do sistema.

use std::sync::{Mutex, MutexGuard};

use egui::{Align2, Color32, CornerRadius, Id, Key, Order, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::json;

use super::home::{self, Overview};
use crate::FigApp;
use crate::theme::{Tokens, regular, semibold};
use crate::widgets;

/// Um destino possível: um projeto da conta (ou o Geral) com o nome da pasta que o representa.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    /// A subpasta de `LegatIA Design` (o programa a escolhe: o nome do projeto sem o que o Windows não aceita).
    pub folder: String,
    pub label: String,
    /// Os nomes (em minúsculas) dos arquivos que o projeto já tem na nuvem.
    pub files: Vec<String>,
}

/// O nome de um arquivo ou pasta que o Windows aceita: sem `<>:"/\|?*` nem controles, espaços juntos, sem ponto ou espaço nas
/// pontas, até 80 letras, e sem os nomes reservados (CON, NUL, COM1…). É a mesma regra do `sanitizeName` do programa.
pub fn sanitize_name(name: &str, fallback: &str) -> String {
    let spaced: String = name.chars().map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { ' ' } else { c }).collect();
    let joined = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut s = joined.trim_matches(|c| c == '.' || c == ' ').to_string();
    if s.chars().count() > 80 {
        s = s.chars().take(80).collect::<String>().trim().to_string();
    }
    let lower = s.to_lowercase();
    let reserved = matches!(lower.as_str(), "con" | "prn" | "aux" | "nul")
        || ((lower.starts_with("com") || lower.starts_with("lpt")) && lower.len() == 4 && lower.as_bytes()[3].is_ascii_digit() && lower.as_bytes()[3] != b'0');
    if s.is_empty() || reserved { fallback.to_string() } else { s }
}

/// Os destinos, na ordem da Home (o projeto mexido por último primeiro, o Geral por último).
pub fn choices(overview: &Overview) -> Vec<Choice> {
    overview
        .groups
        .iter()
        .map(|g| {
            let general = g.id.is_none();
            let folder = if !g.folder.is_empty() { g.folder.clone() } else if general { "Geral".into() } else { sanitize_name(&g.name, "Projeto") };
            let label = if general { "Geral (sem projeto)".to_string() } else { g.name.trim().to_string() };
            Choice { folder, label, files: g.files.iter().map(|f| f.name.to_lowercase()).collect() }
        })
        .collect()
}

/// O projeto aparece numa busca por `filter` (já em minúsculas, vazio: todos)?
pub fn shown(choice: &Choice, filter: &str) -> bool {
    filter.is_empty() || choice.label.to_lowercase().contains(filter)
}

/// O arquivo que o quadro criaria: a pasta do projeto e o nome limpo, ou o motivo de não dar.
pub fn target(choice: &Choice, name: &str) -> Result<(String, String), String> {
    let file = sanitize_name(name, "");
    if file.is_empty() {
        return Err("Escreva um nome para o arquivo.".into());
    }
    if choice.files.iter().any(|existing| *existing == file.to_lowercase()) {
        return Err(format!("“{file}” já existe em {}. Escolha outro nome.", choice.label));
    }
    Ok((choice.folder.clone(), file))
}

struct Dialog {
    name: String,
    filter: String,
    /// A pasta do projeto escolhido.
    selected: Option<String>,
    error: Option<String>,
    focus: bool,
}

static DIALOG: Mutex<Option<Dialog>> = Mutex::new(None);
/// O projeto que a pessoa escolheu na Home ao criar um arquivo ("Outros projetos"): o quadro já abre nele.
static PREFER: Mutex<Option<String>> = Mutex::new(None);

fn dialog() -> MutexGuard<'static, Option<Dialog>> {
    DIALOG.lock().unwrap_or_else(|e| e.into_inner())
}

/// O próximo "Salvar na LegatIA" já vem com este projeto marcado (a pasta dele).
pub fn prefer_project(folder: &str) {
    *PREFER.lock().unwrap_or_else(|e| e.into_inner()) = Some(folder.to_string());
}

/// O quadro está aberto?
pub fn is_open() -> bool {
    dialog().is_some()
}

/// Abre o quadro para o arquivo da frente. Falso quando não há conta ligada (o programa não respondeu): aí vale o seletor do sistema.
pub fn open(app: &FigApp) -> bool {
    let Some(overview) = home::current().filter(|o| o.state == "ok") else { return false };
    let all = choices(&overview);
    let preferred = PREFER.lock().unwrap_or_else(|e| e.into_inner()).take().filter(|p| all.iter().any(|c| c.folder == *p));
    let name = app.session.doc().map(|d| d.name.trim().to_string()).unwrap_or_default();
    let name = if name.is_empty() || name.eq_ignore_ascii_case("untitled") { "Sem título".to_string() } else { name };
    *dialog() = Some(Dialog { name, filter: String::new(), selected: preferred, error: None, focus: true });
    true
}

enum Action {
    Save,
    Cancel,
    Elsewhere,
}

fn pill(ui: &mut Ui, t: &Tokens, label: &str, primary: bool, enabled: bool) -> bool {
    let font = semibold(12.0);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font.clone(), Color32::WHITE);
    let (r, resp) = ui.allocate_exact_size(vec2(galley.size().x + 36.0, 34.0), if enabled { Sense::click() } else { Sense::hover() });
    let fill = match (primary, enabled) {
        (true, true) => if resp.hovered() { crate::legatia::BLUE_400 } else { crate::legatia::BLUE_500 },
        (true, false) => Color32::from_white_alpha(14),
        (false, _) => Color32::from_white_alpha(if resp.hovered() { 22 } else { 12 }),
    };
    ui.painter().rect_filled(r, CornerRadius::same(17), fill);
    let ink = if primary && enabled { Color32::WHITE } else if enabled { t.text } else { t.text_secondary };
    ui.painter().text(r.center(), Align2::CENTER_CENTER, label, font, ink);
    if enabled {
        crate::a11y::button(&resp, label);
    }
    enabled && resp.clicked()
}

/// Desenha o quadro (a cada quadro, enquanto aberto) e faz o que a pessoa escolheu.
pub fn show(app: &mut FigApp, ctx: &egui::Context) {
    let Some(mut d) = dialog().take() else { return };
    let Some(overview) = home::current().filter(|o| o.state == "ok") else { return };
    let all = choices(&overview);
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    // O fundo escurece e segura os cliques: o quadro é modal
    egui::Area::new(Id::new("legatia_save_backdrop")).order(Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
        let (r, _) = ui.allocate_exact_size(screen.size(), Sense::click_and_drag());
        ui.painter().rect_filled(r, CornerRadius::ZERO, Color32::from_black_alpha(150));
    });
    let mut action: Option<Action> = None;
    egui::Area::new(Id::new("legatia_save_card")).order(Order::Foreground).anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0)).show(ctx, |ui| {
        let frame = egui::Frame::NONE.fill(Color32::from_rgb(15, 19, 30)).corner_radius(CornerRadius::same(22)).inner_margin(egui::Margin::same(24));
        let shown_frame = frame.show(ui, |ui| {
            ui.set_width(440.0);
            ui.label(egui::RichText::new("Salvar na LegatIA").font(semibold(17.0)).color(t.text));
            ui.add_space(4.0);
            let offline = !overview.online;
            let subtitle = if offline {
                "Sem conexão: o arquivo fica salvo neste PC, na pasta do projeto, e vai para a nuvem quando a conexão voltar."
            } else {
                "Escolha o projeto e o nome. O arquivo vai para a nuvem em seguida."
            };
            ui.label(egui::RichText::new(subtitle).font(regular(11.5)).color(if offline { t.warning } else { t.text_secondary }));
            ui.add_space(16.0);
            ui.label(egui::RichText::new("Nome do arquivo").font(semibold(11.5)).color(t.text_secondary));
            ui.add_space(6.0);
            let name_id = Id::new("legatia_save_name");
            let out = ui.add(egui::TextEdit::singleline(&mut d.name).id(name_id).hint_text("Sem título").font(regular(13.0)).desired_width(f32::INFINITY));
            if d.focus {
                out.request_focus();
                d.focus = false;
            }
            if out.changed() {
                d.error = None;
            }
            let enter = out.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            ui.add_space(14.0);
            ui.label(egui::RichText::new("Projeto").font(semibold(11.5)).color(t.text_secondary));
            ui.add_space(6.0);
            if all.len() > 6 {
                ui.add(egui::TextEdit::singleline(&mut d.filter).hint_text("Buscar projeto").font(regular(12.0)).desired_width(f32::INFINITY));
                ui.add_space(6.0);
            }
            let filter = d.filter.trim().to_lowercase();
            egui::ScrollArea::vertical().max_height(220.0).auto_shrink([false, true]).show(ui, |ui| {
                let mut any = false;
                for choice in all.iter().filter(|c| shown(c, &filter)) {
                    any = true;
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
                    let on = d.selected.as_deref() == Some(choice.folder.as_str());
                    if on {
                        ui.painter().rect_filled(r, CornerRadius::same(10), Color32::from_rgba_unmultiplied(0x00, 0x6F, 0xFE, 60));
                    } else if resp.hovered() {
                        ui.painter().rect_filled(r, CornerRadius::same(10), Color32::from_white_alpha(14));
                    }
                    let dot = pos2(r.min.x + 16.0, r.center().y);
                    ui.painter().circle_stroke(dot, 6.0, Stroke::new(1.5, if on { crate::legatia::BLUE_400 } else { t.text_secondary }));
                    if on {
                        ui.painter().circle_filled(dot, 3.0, crate::legatia::BLUE_400);
                    }
                    let note = match choice.files.len() {
                        0 => String::new(),
                        1 => "1 arquivo".to_string(),
                        n => format!("{n} arquivos"),
                    };
                    let note_w = if note.is_empty() { 0.0 } else { 90.0 };
                    let text = widgets::elide_font(ui, &choice.label, r.width() - 44.0 - note_w, regular(12.5));
                    ui.painter().text(pos2(r.min.x + 32.0, r.center().y), Align2::LEFT_CENTER, text, regular(12.5), t.text);
                    if !note.is_empty() {
                        ui.painter().text(pos2(r.max.x - 12.0, r.center().y), Align2::RIGHT_CENTER, note, regular(11.0), t.text_secondary);
                    }
                    crate::a11y::button(&resp, &choice.label);
                    if resp.clicked() {
                        d.selected = Some(choice.folder.clone());
                        d.error = None;
                    }
                }
                if !any {
                    ui.label(egui::RichText::new("Nenhum projeto com esse nome.").font(regular(11.5)).color(t.text_secondary));
                }
            });
            if let Some(error) = &d.error {
                ui.add_space(10.0);
                ui.label(egui::RichText::new(error).font(regular(11.5)).color(t.danger));
            }
            ui.add_space(18.0);
            let ready = !d.name.trim().is_empty() && d.selected.is_some();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = vec2(10.0, 0.0);
                let link = ui.add(egui::Label::new(egui::RichText::new("Salvar só neste PC…").font(regular(11.5)).color(t.text_secondary)).sense(Sense::click()));
                if link.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    action = Some(Action::Elsewhere);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if pill(ui, &t, if offline { "Salvar neste PC" } else { "Salvar" }, true, ready) || (enter && ready) {
                        action = Some(Action::Save);
                    }
                    if pill(ui, &t, "Cancelar", false, true) {
                        action = Some(Action::Cancel);
                    }
                });
            });
            if d.selected.is_none() && !d.name.trim().is_empty() {
                ui.add_space(6.0);
                ui.label(egui::RichText::new("Escolha um projeto para salvar.").font(regular(10.5)).color(t.icon_secondary));
            }
        });
        let rect: Rect = shown_frame.response.rect;
        home::ring(ui, rect, 22.0, 1.0);
    });
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
        action = Some(Action::Cancel);
    }
    match action {
        None => *dialog() = Some(d),
        Some(Action::Cancel) => {}
        Some(Action::Elsewhere) => app.save_as_dialog(),
        Some(Action::Save) => {
            let choice = d.selected.as_ref().and_then(|f| all.iter().find(|c| c.folder == *f)).cloned();
            let result = match choice {
                Some(choice) => save(app, ctx, &overview, &choice, &d.name),
                None => Err("Escolha um projeto para salvar.".to_string()),
            };
            match result {
                Ok(()) => {}
                Err(error) => {
                    d.error = Some(error);
                    *dialog() = Some(d);
                }
            }
        }
    }
}

/// Grava o arquivo da frente em `LegatIA Design\<pasta>\<nome>.figcraft` e manda enviar.
fn save(app: &mut FigApp, ctx: &egui::Context, overview: &Overview, choice: &Choice, name: &str) -> Result<(), String> {
    let (folder, file) = target(choice, name)?;
    let relative = format!("{folder}/{file}.figcraft");
    let path = home::local_path(&overview.root, &relative).ok_or("Não consegui montar o caminho do arquivo.")?;
    if path.exists() {
        return Err(format!("“{file}” já existe na pasta de {}. Escolha outro nome.", choice.label));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Não consegui criar a pasta do projeto: {e}"))?;
    }
    if app.run_user("file.saveAs", json!({"path": path.to_string_lossy()})).is_none() {
        return Err("Não consegui salvar o arquivo. Veja o aviso na tela.".into());
    }
    if overview.online {
        super::sync::send_now(path.to_string_lossy().into_owned(), ctx);
        app.toast(format!("Salvo em {}. Enviando para a nuvem…", choice.label));
    } else {
        // Sem conexão: fica na pasta do projeto e o programa cria o arquivo na nuvem quando ela voltar
        app.toast(format!("Sem conexão: salvo neste PC, em {}. Vai para a nuvem quando a conexão voltar.", choice.label));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::legatia::home::{FileInfo, Group};

    fn overview() -> Overview {
        Overview {
            state: "ok".into(),
            root: "C:/x".into(),
            groups: vec![
                Group { id: Some(2), name: "Scalefy 2.0".into(), folder: "Scalefy 2.0".into(), files: vec![FileInfo { name: "Dashboard".into(), ..Default::default() }] },
                Group { id: Some(3), name: "Nerd pagamentos ".into(), folder: String::new(), files: vec![] },
                Group { id: None, name: "Geral".into(), folder: "Geral".into(), files: vec![FileInfo { name: "Id vision".into(), ..Default::default() }] },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn names_follow_the_programs_rule_for_windows() {
        assert_eq!(sanitize_name("Landing: v2 / final?", "x"), "Landing v2 final");
        assert_eq!(sanitize_name("  .Oi.  ", "x"), "Oi");
        assert_eq!(sanitize_name("a    b", "x"), "a b");
        assert_eq!(sanitize_name("CON", "Sem nome"), "Sem nome");
        assert_eq!(sanitize_name("com1", "Sem nome"), "Sem nome");
        assert_eq!(sanitize_name("com0", "Sem nome"), "com0");
        assert_eq!(sanitize_name("***", "Sem nome"), "Sem nome");
        assert_eq!(sanitize_name("", "Sem nome"), "Sem nome");
        assert_eq!(sanitize_name(&"a".repeat(120), "x").chars().count(), 80);
        assert_eq!(sanitize_name("Confido · Novo Frontend", "x"), "Confido · Novo Frontend");
    }

    #[test]
    fn the_choices_come_in_the_homes_order_with_the_folder_the_program_uses() {
        let all = choices(&overview());
        assert_eq!(all.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(), ["Scalefy 2.0", "Nerd pagamentos", "Geral (sem projeto)"]);
        assert_eq!(all[0].folder, "Scalefy 2.0");
        // Sem a pasta do programa (um programa antigo), a mesma regra dele
        assert_eq!(all[1].folder, "Nerd pagamentos");
        assert_eq!(all[2].folder, "Geral");
        assert_eq!(all[0].files, ["dashboard"]);
    }

    #[test]
    fn the_search_looks_in_the_project_names() {
        let all = choices(&overview());
        assert!(shown(&all[0], "") && shown(&all[0], "scale") && !shown(&all[0], "nerd"));
        assert!(shown(&all[1], "nerd"));
    }

    #[test]
    fn a_file_needs_a_name_and_not_one_the_project_already_has() {
        let all = choices(&overview());
        assert_eq!(target(&all[0], "Landing: nova"), Ok(("Scalefy 2.0".to_string(), "Landing nova".to_string())));
        assert!(target(&all[0], "   ").is_err());
        assert!(target(&all[0], "dashboard").unwrap_err().contains("já existe"));
        // O mesmo nome em outro projeto serve
        assert!(target(&all[1], "Dashboard").is_ok());
    }
}
