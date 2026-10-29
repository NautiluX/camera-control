use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, env, fs, io, path::PathBuf};
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Profile {
    pub watch: bool,
    pub desired: BTreeMap<u32, i32>,
    #[serde(default)]
    pub preview: crate::preview::Options,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Settings {
    pub selected: Option<PathBuf>,
    pub cameras: BTreeMap<PathBuf, Profile>,
}
fn path() -> io::Result<PathBuf> {
    let base = env::var_os("XDG_CONFIG_HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))
        .ok_or_else(|| io::Error::other("No configuration directory is available"))?;
    Ok(base.join("camera-control/settings.json"))
}
impl Settings {
    pub fn load() -> io::Result<Self> {
        match fs::read(path()?) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }
    pub fn save(&self) -> io::Result<()> {
        let path = path()?;
        fs::create_dir_all(path.parent().unwrap())?;
        let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
        fs::write(
            &temporary,
            serde_json::to_vec_pretty(self).map_err(io::Error::other)?,
        )?;
        fs::rename(temporary, path)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn existing_profiles_receive_preview_defaults() {
        let profile: Profile = serde_json::from_str(r#"{"watch":true,"desired":{}}"#).unwrap();
        assert!(profile.watch);
        assert_eq!(profile.preview, crate::preview::Options::default());
    }
    #[test]
    fn camera_profiles_round_trip_without_losing_auto_mode() {
        let mut settings = Settings::default();
        settings.cameras.insert(
            PathBuf::from("/dev/v4l/by-id/test"),
            Profile {
                watch: true,
                preview: crate::preview::Options {
                    resolution: crate::preview::Resolution {
                        width: 1920,
                        height: 1080,
                    },
                    fps: 24,
                    mirror: true,
                },
                desired: [
                    (crate::camera::AUTO_WB, 1),
                    (crate::camera::TEMPERATURE, 5500),
                ]
                .into(),
            },
        );
        let restored: Settings =
            serde_json::from_slice(&serde_json::to_vec(&settings).unwrap()).unwrap();
        let saved = restored.cameras.values().next().unwrap();
        assert!(saved.watch);
        assert_eq!(
            saved.preview.resolution,
            crate::preview::Resolution {
                width: 1920,
                height: 1080
            }
        );
        assert_eq!(saved.preview.fps, 24);
        assert!(saved.preview.mirror);
        assert_eq!(
            restored.cameras.values().next().unwrap().desired[&crate::camera::AUTO_WB],
            1
        );
    }
}
