//! LegatIA: a foto de perfil de cada agente de IA.
//!
//! Quando um agente trabalha no arquivo (Claude, Codex, GPT, Antigravity, Gemini…), a barra de pessoas e a animação de demonstração
//! mostram a marca dele no lugar de uma letra. Quem é o agente vem do nome que o próprio cliente MCP diz ao conectar (e não de quem
//! acionou): `claude-code` é o Claude, `codex-mcp-client` é o Codex, e assim por diante. Um nome que não conhecemos fica com a letra.
//!
//! As imagens (`assets/legatia/agents/*.png`, 96 px) são o símbolo de cada marca em branco num círculo da cor dela. Os símbolos do Claude,
//! da OpenAI (Codex e GPT) e do Gemini são do conjunto aberto simple-icons (CC0, as marcas seguem sendo de quem as criou); o do
//! Antigravity não está nesse conjunto e é um "A" desenhado por nós, até haver o arquivo oficial.

use egui::{Color32, Context, Painter, Pos2, Rect, TextureHandle, TextureOptions, pos2, vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Brand {
    Claude,
    Codex,
    Gpt,
    Antigravity,
    Gemini,
}

/// Qual marca é o agente de nome `name` (o que o cliente MCP disse ao conectar), ou `None` se não conhecemos.
pub fn brand_of(name: &str) -> Option<Brand> {
    let n = name.to_lowercase();
    let has = |s: &str| n.contains(s);
    if has("claude") || has("anthropic") {
        Some(Brand::Claude)
    } else if has("codex") {
        Some(Brand::Codex)
    } else if has("antigravity") || n == "agy" || has("agy ") {
        Some(Brand::Antigravity)
    } else if has("gemini") {
        Some(Brand::Gemini)
    } else if has("gpt") || has("openai") {
        Some(Brand::Gpt)
    } else {
        None
    }
}

fn bytes(b: Brand) -> &'static [u8] {
    match b {
        Brand::Claude => include_bytes!("../../../../assets/legatia/agents/claude.png"),
        Brand::Codex => include_bytes!("../../../../assets/legatia/agents/codex.png"),
        Brand::Gpt => include_bytes!("../../../../assets/legatia/agents/gpt.png"),
        Brand::Antigravity => include_bytes!("../../../../assets/legatia/agents/antigravity.png"),
        Brand::Gemini => include_bytes!("../../../../assets/legatia/agents/gemini.png"),
    }
}

/// A textura da marca (decodificada uma vez e guardada no contexto).
fn texture(ctx: &Context, b: Brand) -> Option<TextureHandle> {
    let id = egui::Id::new(("legatia_agent_mark", b as u8));
    if let Some(t) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return Some(t);
    }
    let rgba = image::load_from_memory(bytes(b)).ok()?.to_rgba8();
    let image = egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw());
    let t = ctx.load_texture(format!("legatia-agent-{b:?}"), image, TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, t.clone()));
    Some(t)
}

/// Pinta a foto do agente `name` num círculo de raio `r` em `center`; `false` se não conhecemos a marca (quem chamou põe a letra).
/// `alpha` (0 a 1) é a opacidade, para quem some aos poucos.
pub fn paint(painter: &Painter, ctx: &Context, center: Pos2, r: f32, name: &str, alpha: f32) -> bool {
    let Some(brand) = brand_of(name) else { return false };
    let Some(tex) = texture(ctx, brand) else { return false };
    let rect = Rect::from_center_size(center, vec2(2.0 * r, 2.0 * r));
    painter.image(tex.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE.gamma_multiply(alpha));
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_client_names_map_to_their_brand() {
        assert_eq!(brand_of("Claude (agent)"), Some(Brand::Claude));
        assert_eq!(brand_of("claude-code"), Some(Brand::Claude));
        assert_eq!(brand_of("claude-ai"), Some(Brand::Claude));
        assert_eq!(brand_of("codex-mcp-client"), Some(Brand::Codex));
        assert_eq!(brand_of("Codex (agent)"), Some(Brand::Codex));
        assert_eq!(brand_of("Antigravity"), Some(Brand::Antigravity));
        assert_eq!(brand_of("agy"), Some(Brand::Antigravity));
        assert_eq!(brand_of("gemini-cli-mcp-client"), Some(Brand::Gemini));
        assert_eq!(brand_of("ChatGPT"), Some(Brand::Gpt));
        assert_eq!(brand_of("openai-mcp"), Some(Brand::Gpt));
        assert_eq!(brand_of("AI agent"), None);
        assert_eq!(brand_of("Cursor"), None);
    }

    #[test]
    fn every_mark_decodes() {
        for b in [Brand::Claude, Brand::Codex, Brand::Gpt, Brand::Antigravity, Brand::Gemini] {
            let img = image::load_from_memory(bytes(b)).expect("a imagem abre").to_rgba8();
            assert_eq!((img.width(), img.height()), (96, 96), "{b:?}");
        }
    }
}
