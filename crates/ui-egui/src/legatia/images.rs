//! LegatIA: as imagens da nuvem na Home e no editor (a foto de perfil de quem edita e a capa de cada projeto).
//!
//! Como o resto, isto não fala com o Supabase: pede ao programa `legatia-sync` (`/native/avatar?user=<id>` e
//! `/native/cover?project=<id>`), que só baixa do Supabase da própria conta. As imagens chegam de uma fila de
//! trabalhadores em segundo plano (a janela nunca espera a rede), são decodificadas e viram textura na primeira vez
//! que alguém as desenha. Uma que não veio (sem foto, sem rede) é pedida de novo depois de um minuto.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

enum Slot {
    /// Já na fila ou sendo baixada.
    Loading,
    /// Baixada e decodificada; vira textura ao ser desenhada.
    Ready(egui::ColorImage),
    Texture(egui::TextureHandle),
    /// Não veio; vale tentar de novo depois de [`RETRY`].
    Missing(Instant),
}

struct Job {
    key: String,
    target: String,
    fit: u32,
    ctx: egui::Context,
}

struct Cache {
    slots: HashMap<String, Slot>,
    queue: Option<Sender<Job>>,
}

static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(|| Mutex::new(Cache { slots: HashMap::new(), queue: None }));

const RETRY: Duration = Duration::from_secs(60);
const WORKERS: usize = 3;

fn cache() -> MutexGuard<'static, Cache> {
    CACHE.lock().unwrap_or_else(|e| e.into_inner())
}

/// O endereço (no programa) da foto de perfil de uma pessoa, ou `None` se o id não parece de usuário.
pub fn avatar_target(user_id: &str) -> Option<String> {
    is_id(user_id).then(|| format!("/native/avatar?user={user_id}"))
}

/// O endereço (no programa) da capa de um projeto.
pub fn cover_target(project_id: i64) -> String {
    format!("/native/cover?project={project_id}")
}

/// Um id de usuário do Supabase (um UUID): só dígitos hexadecimais e hífens, 36 caracteres.
fn is_id(id: &str) -> bool {
    id.len() == 36 && id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}

fn spawn_workers() -> Sender<Job> {
    let (tx, rx) = channel::<Job>();
    let rx: Arc<Mutex<Receiver<Job>>> = Arc::new(Mutex::new(rx));
    for n in 0..WORKERS {
        let rx = rx.clone();
        let _ = std::thread::Builder::new().name(format!("legatia-images-{n}")).spawn(move || {
            loop {
                let job = match rx.lock().unwrap_or_else(|e| e.into_inner()).recv() {
                    Ok(job) => job,
                    Err(_) => return,
                };
                let image = super::sync::image_bytes(&job.target)
                    .and_then(|bytes| figcraft_format::thumbnail::decode_thumbnail(&bytes, job.fit))
                    .map(|p| egui::ColorImage::from_rgba_unmultiplied([p.width as usize, p.height as usize], &p.rgba));
                let mut c = cache();
                c.slots.insert(job.key, image.map_or_else(|| Slot::Missing(Instant::now()), Slot::Ready));
                drop(c);
                job.ctx.request_repaint();
            }
        });
    }
    tx
}

/// A imagem como textura, ou `None` enquanto não chegou (ela é pedida na primeira chamada). `key` identifica a imagem
/// (`avatar:<id>`, `cover:<id>`), `target` é o endereço no programa e `fit`, o maior lado em pixels depois de decodificada.
pub fn texture(ctx: &egui::Context, key: &str, target: &str, fit: u32) -> Option<egui::TextureHandle> {
    if cfg!(test) {
        return None;
    }
    let mut c = cache();
    match c.slots.get(key) {
        Some(Slot::Texture(handle)) => return Some(handle.clone()),
        Some(Slot::Ready(_)) => {
            if let Some(Slot::Ready(image)) = c.slots.remove(key) {
                let handle = ctx.load_texture(format!("legatia-{key}"), image, egui::TextureOptions::LINEAR);
                c.slots.insert(key.to_string(), Slot::Texture(handle.clone()));
                return Some(handle);
            }
            return None;
        }
        Some(Slot::Loading) => return None,
        Some(Slot::Missing(at)) if at.elapsed() < RETRY => return None,
        _ => {}
    }
    c.slots.insert(key.to_string(), Slot::Loading);
    let queue = c.queue.get_or_insert_with(spawn_workers).clone();
    drop(c);
    if queue.send(Job { key: key.to_string(), target: target.to_string(), fit, ctx: ctx.clone() }).is_err() {
        cache().slots.insert(key.to_string(), Slot::Missing(Instant::now()));
    }
    None
}

/// A foto de perfil de alguém (por id de usuário), ou `None` enquanto não chegou ou se a pessoa não tem foto.
pub fn avatar(ctx: &egui::Context, user_id: &str) -> Option<egui::TextureHandle> {
    let target = avatar_target(user_id)?;
    texture(ctx, &format!("avatar:{user_id}"), &target, 96)
}

/// O endereço (no programa) da foto de alguém pelo nome que aparece na sessão ao vivo ou num comentário (o nome da conta
/// da LegatIA). `None` para um nome vazio ou comprido demais.
pub fn avatar_name_target(name: &str) -> Option<String> {
    let name = name.trim();
    (!name.is_empty() && name.chars().count() <= 80 && !name.chars().any(char::is_control))
        .then(|| format!("/native/avatar?name={}", super::sync::encode_query(name)))
}

/// A foto de alguém pelo nome (o cursor de quem está na sessão ao vivo, o autor de um comentário), ou `None` enquanto não
/// chegou, se o nome não é de ninguém da LegatIA ou se a pessoa não tem foto.
pub fn avatar_by_name(ctx: &egui::Context, name: &str) -> Option<egui::TextureHandle> {
    let target = avatar_name_target(name)?;
    texture(ctx, &format!("avatar-name:{}", name.trim()), &target, 96)
}

/// A capa de um projeto (por id), ou `None` enquanto não chegou ou se o projeto não tem capa.
pub fn cover(ctx: &egui::Context, project_id: i64) -> Option<egui::TextureHandle> {
    texture(ctx, &format!("cover:{project_id}"), &cover_target(project_id), 640)
}

/// O recorte que preenche um retângulo de proporção `aspect` (largura/altura) com uma imagem de `size`, sem esticar:
/// as coordenadas (0 a 1) da parte central que fica à mostra.
pub fn cover_uv(size: egui::Vec2, aspect: f32) -> egui::Rect {
    let (w, h) = (size.x.max(1.0), size.y.max(1.0));
    let (u, v) = if w / h > aspect { ((h * aspect) / w, 1.0) } else { (1.0, (w / aspect.max(0.01)) / h) };
    egui::Rect::from_center_size(egui::pos2(0.5, 0.5), egui::vec2(u.min(1.0), v.min(1.0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_becomes_an_address_only_when_it_looks_like_a_name() {
        assert_eq!(avatar_name_target("João Victor").as_deref(), Some("/native/avatar?name=Jo%C3%A3o%20Victor"));
        assert_eq!(avatar_name_target("  "), None);
        assert_eq!(avatar_name_target(&"x".repeat(81)), None);
        assert_eq!(avatar_name_target("a
b"), None);
    }

    #[test]
    fn only_a_user_id_becomes_an_address() {
        let id = "22222222-2222-4222-8222-222222222222";
        assert_eq!(avatar_target(id).as_deref(), Some("/native/avatar?user=22222222-2222-4222-8222-222222222222"));
        assert_eq!(avatar_target("../../etc/passwd"), None);
        assert_eq!(avatar_target(""), None);
        assert_eq!(avatar_target("22222222-2222-4222-8222-22222222222&"), None);
        assert_eq!(cover_target(7), "/native/cover?project=7");
    }

    #[test]
    fn the_cover_fills_the_box_without_stretching() {
        // Uma capa larga numa caixa quadrada: corta dos lados
        let wide = cover_uv(egui::vec2(200.0, 100.0), 1.0);
        assert!((wide.width() - 0.5).abs() < 1e-5 && (wide.height() - 1.0).abs() < 1e-5);
        // Uma alta numa caixa larga: corta em cima e embaixo
        let tall = cover_uv(egui::vec2(100.0, 200.0), 2.0);
        assert!((tall.width() - 1.0).abs() < 1e-5 && (tall.height() - 0.25).abs() < 1e-5);
        // Mesma proporção: a imagem inteira
        let same = cover_uv(egui::vec2(300.0, 100.0), 3.0);
        assert!((same.width() - 1.0).abs() < 1e-5 && (same.height() - 1.0).abs() < 1e-5);
    }
}
