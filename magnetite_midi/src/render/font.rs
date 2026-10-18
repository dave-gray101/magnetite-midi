//! Loading a SoundFont once for everything that uses the file.

use super::RenderError;
use crate::SoundFont;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};

type Cache = Mutex<HashMap<PathBuf, Weak<SoundFont>>>;

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// Load a SoundFont, or share the one already loaded from that file: sources that use one font hold one copy. The copy is freed when the
/// last of them lets go.
pub fn load_soundfont(path: &Path) -> Result<Arc<SoundFont>, RenderError> {
    let key = path
        .canonicalize()
        .map_err(|e| RenderError(format!("cannot open the SoundFont {}: {e}", path.display())))?;
    if let Some(font) = cache().lock().unwrap().get(&key).and_then(Weak::upgrade) {
        return Ok(font);
    }
    let file = std::fs::File::open(&key)
        .map_err(|e| RenderError(format!("cannot open the SoundFont {}: {e}", path.display())))?;
    let font = SoundFont::new(&mut std::io::BufReader::new(file)).map_err(|e| {
        let sf3 = format!("{e:?}").contains("UnsupportedSampleFormat");
        RenderError(if sf3 {
            format!(
                "{} is a compressed SoundFont (SF3); only SF2 is supported",
                path.display()
            )
        } else {
            format!(
                "{} is not a SoundFont this synthesizer can read ({e})",
                path.display()
            )
        })
    })?;
    let font = Arc::new(font);
    let mut c = cache().lock().unwrap();
    c.retain(|_, w| w.strong_count() > 0);
    c.insert(key, Arc::downgrade(&font));
    Ok(font)
}
