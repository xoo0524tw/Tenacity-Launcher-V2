use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::AsyncWriteExt;

const REPO: &str = "xoo0524tw/Tenacity-Launcher";
const UA: &str = "Tenacity-Launcher-Tauri";
const JAR: &str = "Tenacity.jar";
const GAME_MAIN: &str = "net.minecraft.client.main.Main";
const MIN_JAR_BYTES: u64 = 1_048_576;

#[cfg(target_os = "macos")]
const MAC_NATIVE_JARS: [&str; 3] = [
    "lwjgl-platform-2.9.4-nightly-20150209-natives-osx.jar",
    "jinput-platform-2.0.5-natives-osx.jar",
    "twitch-platform-6.5-natives-osx.jar",
];

#[derive(Serialize, Clone)]
struct ReleaseAsset {
    name: String,
    size: u64,
    url: String,
}

#[derive(Serialize, Clone)]
struct ReleaseInfo {
    tag: String,
    name: String,
    published_at: String,
    asset: Option<ReleaseAsset>,
}

#[derive(Serialize, Clone)]
struct InstalledVersion {
    tag: String,
    size: u64,
}

#[derive(Serialize, Clone)]
struct DownloadProgress {
    tag: String,
    downloaded: u64,
    total: u64,
}

#[derive(serde::Deserialize)]
struct GhAsset {
    name: String,
    size: u64,
    browser_download_url: String,
}

#[derive(serde::Deserialize)]
struct GhRelease {
    tag_name: String,
    name: Option<String>,
    published_at: Option<String>,
    assets: Vec<GhAsset>,
}

fn github_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(UA)
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))
}

fn bundled_jre_candidates() -> &'static [&'static str] {
    &[
        #[cfg(target_os = "windows")]
        "jre/bin/java.exe",
        #[cfg(target_os = "linux")]
        "jrex64-linux/bin/java",
        #[cfg(target_os = "macos")]
        "jrex64-mac/bin/java",
    ]
}

fn is_runtime_dir(dir: &Path) -> bool {
    if !dir.join("libs").is_dir() {
        return false;
    }
    #[cfg(target_os = "macos")]
    {
        true
    }
    #[cfg(not(target_os = "macos"))]
    {
        bundled_jre_candidates()
            .iter()
            .any(|rel| dir.join(rel).is_file())
    }
}

fn find_files_dir(app: &AppHandle) -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent()?.to_path_buf();
        for _ in 0..4 {
            let candidate = dir.join("files");
            if is_runtime_dir(&candidate) {
                return Some(candidate);
            }
            if !dir.pop() {
                break;
            }
        }
    }
    if let Ok(res) = app.path().resource_dir() {
        let candidate = res.join("files");
        if is_runtime_dir(&candidate) {
            return Some(candidate);
        }
    }
    if let Some(docs) = dirs_documents() {
        let candidate = docs.join("Tenacity-Launcher").join("files");
        if is_runtime_dir(&candidate) {
            return Some(candidate);
        }
    }
    if let Ok(data) = app.path().app_data_dir() {
        let candidate = data.join("files");
        if is_runtime_dir(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn dirs_documents() -> Option<PathBuf> {
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        return Some(PathBuf::from(profile).join("Documents"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        return Some(PathBuf::from(home).join("Documents"));
    }
    None
}

fn data_root(app: &AppHandle) -> PathBuf {
    if let Some(files) = find_files_dir(app) {
        if let Some(parent) = files.parent() {
            return parent.to_path_buf();
        }
    }
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn versions_dir(app: &AppHandle) -> PathBuf {
    data_root(app).join("versions")
}

fn pick_asset<'a>(assets: &'a [GhAsset]) -> Option<&'a GhAsset> {
    assets
        .iter()
        .find(|a| a.name == JAR)
        .or_else(|| {
            assets
                .iter()
                .find(|a| a.name.ends_with(".jar") && a.name.contains("Tenacity"))
        })
}

#[tauri::command]
async fn list_releases() -> Result<Vec<ReleaseInfo>, String> {
    let client = github_client()?;
    let res = client
        .get(format!(
            "https://api.github.com/repos/{REPO}/releases?per_page=100"
        ))
        .send()
        .await
        .map_err(|e| format!("Failed to reach GitHub: {e}"))?;
    if !res.status().is_success() {
        return Err(format!("GitHub API error: {}", res.status()));
    }
    let releases: Vec<GhRelease> = res
        .json()
        .await
        .map_err(|e| format!("Failed to parse GitHub response: {e}"))?;

    Ok(releases
        .into_iter()
        .map(|r| {
            let asset = pick_asset(&r.assets).map(|a| ReleaseAsset {
                name: a.name.clone(),
                size: a.size,
                url: a.browser_download_url.clone(),
            });
            ReleaseInfo {
                tag: r.tag_name,
                name: r.name.unwrap_or_default(),
                published_at: r.published_at.unwrap_or_default(),
                asset,
            }
        })
        .collect())
}

#[tauri::command]
async fn install_version(app: AppHandle, tag: String) -> Result<(), String> {
    let client = github_client()?;
    let res = client
        .get(format!(
            "https://api.github.com/repos/{REPO}/releases/tags/{tag}"
        ))
        .send()
        .await
        .map_err(|e| format!("Failed to reach GitHub: {e}"))?;
    if !res.status().is_success() {
        return Err(format!("Release {tag} not found ({})", res.status()));
    }
    let release: GhRelease = res
        .json()
        .await
        .map_err(|e| format!("Failed to parse release: {e}"))?;
    let asset = pick_asset(&release.assets)
        .ok_or_else(|| format!("Release {tag} has no Tenacity.jar asset"))?;

    let version_dir = versions_dir(&app).join(&tag);
    fs::create_dir_all(&version_dir).map_err(|e| format!("Failed to create folder: {e}"))?;
    let tmp_path = version_dir.join(format!("{JAR}.part"));
    let final_path = version_dir.join(JAR);

    let mut resp = client
        .get(&asset.browser_download_url)
        .send()
        .await
        .map_err(|e| format!("Download failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Download failed: {}", resp.status()));
    }

    let total = resp.content_length().unwrap_or(0);
    let mut file = tokio::fs::File::create(&tmp_path)
        .await
        .map_err(|e| format!("Failed to create temp file: {e}"))?;
    let mut downloaded: u64 = 0;
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| format!("Download interrupted: {e}"))?
    {
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("Failed to write file: {e}"))?;
        downloaded += chunk.len() as u64;
        let _ = app.emit(
            "download-progress",
            DownloadProgress {
                tag: tag.clone(),
                downloaded,
                total,
            },
        );
    }
    file.flush()
        .await
        .map_err(|e| format!("Failed to flush file: {e}"))?;
    drop(file);

    let size = fs::metadata(&tmp_path)
        .map_err(|e| format!("Failed to check download: {e}"))?
        .len();
    if size < MIN_JAR_BYTES {
        let _ = fs::remove_file(&tmp_path);
        return Err("Downloaded jar is unexpectedly small — please retry.".into());
    }
    fs::rename(&tmp_path, &final_path).map_err(|e| format!("Failed to finalize file: {e}"))?;
    let _ = app.emit("versions-changed", ());
    Ok(())
}

#[tauri::command]
fn list_installed(app: AppHandle) -> Result<Vec<InstalledVersion>, String> {
    let dir = versions_dir(&app);
    let mut out = Vec::new();
    if dir.exists() {
        for entry in fs::read_dir(&dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if !entry.file_type().map_err(|e| e.to_string())?.is_dir() {
                continue;
            }
            let jar = entry.path().join(JAR);
            if jar.exists() {
                out.push(InstalledVersion {
                    tag: entry.file_name().to_string_lossy().into_owned(),
                    size: fs::metadata(&jar).map_err(|e| e.to_string())?.len(),
                });
            }
        }
    }
    out.sort_by(|a, b| b.tag.cmp(&a.tag));
    Ok(out)
}

#[tauri::command]
fn delete_version(app: AppHandle, tag: String) -> Result<(), String> {
    let dir = versions_dir(&app).join(&tag);
    if dir.exists() {
        fs::remove_dir_all(&dir).map_err(|e| format!("Failed to delete {tag}: {e}"))?;
    }
    let _ = app.emit("versions-changed", ());
    Ok(())
}

#[cfg(target_os = "macos")]
fn resolve_java(files_dir: &Path) -> Result<PathBuf, String> {
    let bundled = files_dir.join("jrex64-mac").join("bin").join("java");
    if bundled.is_file() {
        return Ok(bundled);
    }
    for candidate in [
        PathBuf::from("/Library/Java/JavaVirtualMachines/jdk1.8.0_202.jdk/Contents/Home/bin/java"),
        PathBuf::from("/Library/Internet Plug-Ins/JavaAppletPlugin.plugin/Contents/Home/bin/java"),
    ] {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    if let Ok(output) = Command::new("/usr/libexec/java_home")
        .args(["-v", "1.8", "-arch", "x86_64"])
        .output()
    {
        if output.status.success() {
            let home = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !home.is_empty() {
                let java = PathBuf::from(home).join("bin").join("java");
                if java.is_file() {
                    return Ok(java);
                }
            }
        }
    }
    Err("Intel (x86_64) Java 8 was not found. Tenacity ships Intel-only macOS native libraries; install an x86_64 Java 8 runtime (e.g. JDK 8 from Adoptium) and try again.".into())
}

#[cfg(target_os = "linux")]
fn resolve_java(files_dir: &Path) -> Result<PathBuf, String> {
    let bundled = files_dir.join("jrex64-linux").join("bin").join("java");
    if bundled.is_file() {
        return Ok(bundled);
    }
    if let Ok(output) = Command::new("java").arg("-version").output() {
        if output.status.success() {
            return Ok(PathBuf::from("java"));
        }
    }
    Err("Java not found. Install Java 8 or place a Linux JRE in files/jrex64-linux/.".into())
}

#[cfg(target_os = "windows")]
fn resolve_java(files_dir: &Path) -> Result<PathBuf, String> {
    let java = files_dir.join("jre").join("bin").join("java.exe");
    if java.is_file() {
        return Ok(java);
    }
    Err("java.exe not found in files/jre/bin/.".into())
}

#[cfg(target_os = "macos")]
fn ensure_macos_natives(files_dir: &Path, save_dir: &Path) -> Result<PathBuf, String> {
    let natives_dir = save_dir.join("natives-macos-x86_64");
    fs::create_dir_all(&natives_dir).map_err(|e| e.to_string())?;
    for jar_name in MAC_NATIVE_JARS {
        let jar = files_dir.join("libs").join(jar_name);
        if !jar.is_file() {
            return Err(format!("Missing required macOS native library: {jar_name}"));
        }
        let status = Command::new("unzip")
            .arg("-oq")
            .arg(&jar)
            .arg("-d")
            .arg(&natives_dir)
            .status()
            .map_err(|e| format!("Failed to run unzip: {e}"))?;
        if !status.success() {
            return Err(format!("Failed to extract {jar_name}"));
        }
    }
    Ok(natives_dir)
}

fn native_dir(files_dir: &Path, save_dir: &Path) -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    {
        ensure_macos_natives(files_dir, save_dir)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(files_dir.join("natives"))
    }
}

#[tauri::command]
async fn launch_game(app: AppHandle, tag: String) -> Result<(), String> {
    let files_dir = find_files_dir(&app)
        .ok_or("Could not locate the files/ runtime folder (JRE, libs, natives).")?;
    let root = data_root(&app);
    let save_dir = root.join("save");
    fs::create_dir_all(&save_dir).map_err(|e| e.to_string())?;

    let java = resolve_java(&files_dir)?;
    let jar = versions_dir(&app).join(&tag).join(JAR);
    if !jar.exists() {
        return Err(format!("Version {tag} is not installed."));
    }

    let natives = native_dir(&files_dir, &save_dir)?;
    let libs = files_dir.join("libs");
    let assets = files_dir.join("assets");

    let sep = if cfg!(target_os = "windows") { ";" } else { ":" };
    let classpath = format!("{}{}{}*", jar.display(), sep, libs.display());

    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = Command::new("arch");
        c.arg("-x86_64").arg(&java);
        c
    };
    #[cfg(not(target_os = "macos"))]
    let mut cmd = Command::new(&java);

    cmd.current_dir(&save_dir)
        .arg("-noverify")
        .arg(format!("-Djava.library.path={}", natives.display()))
        .arg("-cp")
        .arg(&classpath)
        .arg(GAME_MAIN)
        .arg("--version")
        .arg("Tenacity")
        .arg("--accessToken")
        .arg("0")
        .arg("--userProperties")
        .arg("{}")
        .arg("--gameDir")
        .arg(save_dir.display().to_string())
        .arg("--assetsDir")
        .arg(assets.display().to_string())
        .arg("--assetIndex")
        .arg("1.8")
        .arg("--width")
        .arg("854")
        .arg("--height")
        .arg("480");

    let child = cmd
        .spawn()
        .map_err(|e| format!("Failed to launch the game: {e}"))?;
    drop(child);
    let _ = app.emit("game-launched", tag);
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            list_releases,
            install_version,
            list_installed,
            delete_version,
            launch_game
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}