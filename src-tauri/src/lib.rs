use serde::Serialize;
use std::collections::HashSet;
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
struct RuntimeInfo {
    files_dir: String,
    java_path: String,
    java_source: String,
    java_version: String,
}

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

#[derive(Serialize, Clone)]
struct GameOutput {
    line: String,
    kind: String,
}

#[derive(Serialize, Clone)]
struct GameExit {
    tag: String,
    code: Option<i32>,
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
    dir.join("libs").is_dir() && dir.join("natives").is_dir()
}

/// The `files/` payload shipped inside the app bundle/installer.
fn bundled_files_dir(app: &AppHandle) -> Option<PathBuf> {
    let candidate = app.path().resource_dir().ok()?.join("files");
    is_runtime_dir(&candidate).then_some(candidate)
}

fn find_files_dir(app: &AppHandle) -> Option<PathBuf> {
    if let Ok(cwd) = std::env::current_dir() {
        let candidate = cwd.join("files");
        if is_runtime_dir(&candidate) {
            return Some(candidate);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent()?.to_path_buf();
        for _ in 0..8 {
            let candidate = dir.join("files");
            if is_runtime_dir(&candidate) {
                return Some(candidate);
            }
            if !dir.pop() {
                break;
            }
        }
    }
    if let Some(candidate) = bundled_files_dir(app) {
        return Some(candidate);
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

fn is_bundled_files(app: &AppHandle, files: &Path) -> bool {
    app.path()
        .resource_dir()
        .map(|res| files.starts_with(res))
        .unwrap_or(false)
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

fn app_data_dir(app: &AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn data_root(app: &AppHandle) -> PathBuf {
    match find_files_dir(app) {
        // The bundled payload lives inside the install folder, which is
        // read-only on macOS/Linux, so user data goes to the app data dir.
        Some(files) if is_bundled_files(app, &files) => app_data_dir(app),
        Some(files) => files
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| app_data_dir(app)),
        None => app_data_dir(app),
    }
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

fn bundled_java(files_dir: &Path) -> Option<PathBuf> {
    bundled_jre_candidates()
        .iter()
        .find_map(|rel| {
            let p = files_dir.join(rel);
            p.is_file().then_some(p)
        })
}

fn java_version(java: &Path) -> Option<String> {
    let out = Command::new(java).arg("-version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stderr);
    text.lines().next().map(|l| l.trim().to_string())
}

fn add_if_file(cands: &mut Vec<PathBuf>, seen: &mut HashSet<PathBuf>, p: PathBuf) {
    if p.is_file() && seen.insert(p.clone()) {
        cands.push(p);
    }
}

#[cfg(target_os = "windows")]
fn system_java_candidates() -> Vec<PathBuf> {
    let mut cands = Vec::new();
    let mut seen = HashSet::new();
    if let Ok(home) = std::env::var("JAVA_HOME") {
        add_if_file(
            &mut cands,
            &mut seen,
            PathBuf::from(home).join("bin").join("java.exe"),
        );
    }
    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            add_if_file(&mut cands, &mut seen, dir.join("java.exe"));
        }
    }
    for base in [
        "C:\\Program Files\\Java",
        "C:\\Program Files (x86)\\Java",
        "C:\\Program Files\\Eclipse Adoptium",
        "C:\\Program Files\\Microsoft",
        "C:\\Program Files\\Common Files\\Oracle\\Java\\javapath",
    ] {
        if let Ok(entries) = fs::read_dir(base) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    add_if_file(&mut cands, &mut seen, p.join("bin").join("java.exe"));
                } else {
                    add_if_file(&mut cands, &mut seen, p);
                }
            }
        }
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        if let Ok(entries) = fs::read_dir(PathBuf::from(local).join("Programs")) {
            for e in entries.flatten() {
                let p = e.path();
                add_if_file(&mut cands, &mut seen, p.join("bin").join("java.exe"));
            }
        }
    }
    cands
}

#[cfg(target_os = "linux")]
fn system_java_candidates() -> Vec<PathBuf> {
    let mut cands = Vec::new();
    let mut seen = HashSet::new();
    if let Ok(home) = std::env::var("JAVA_HOME") {
        add_if_file(&mut cands, &mut seen, PathBuf::from(home).join("bin").join("java"));
    }
    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            add_if_file(&mut cands, &mut seen, dir.join("java"));
        }
    }
    for base in ["/usr/lib/jvm", "/opt"] {
        if let Ok(entries) = fs::read_dir(base) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    add_if_file(&mut cands, &mut seen, p.join("bin").join("java"));
                }
            }
        }
    }
    cands
}

#[cfg(target_os = "macos")]
fn system_java_candidates() -> Vec<PathBuf> {
    let mut cands = Vec::new();
    let mut seen = HashSet::new();
    if let Ok(home) = std::env::var("JAVA_HOME") {
        add_if_file(&mut cands, &mut seen, PathBuf::from(home).join("bin").join("java"));
    }
    for base in [
        "/Library/Java/JavaVirtualMachines",
        "/Library/Internet Plug-Ins/JavaAppletPlugin.plugin/Contents/Home/bin",
    ] {
        if let Ok(entries) = fs::read_dir(base) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    add_if_file(&mut cands, &mut seen, p.join("Contents").join("Home").join("bin").join("java"));
                    add_if_file(&mut cands, &mut seen, p.join("bin").join("java"));
                } else {
                    add_if_file(&mut cands, &mut seen, p);
                }
            }
        }
    }
    cands
}

fn detect_system_java() -> Option<(PathBuf, String)> {
    let mut best: Option<(PathBuf, String)> = None;
    for cand in system_java_candidates() {
        if let Some(ver) = java_version(&cand) {
            let is8 = ver.contains("1.8");
            match &best {
                None => best = Some((cand, ver)),
                Some((_, v)) if is8 && !v.contains("1.8") => best = Some((cand, ver)),
                _ => {}
            }
        }
    }
    best
}

fn resolve_java(app: &AppHandle) -> Result<(PathBuf, String), String> {
    if let Some(files) = find_files_dir(app) {
        if let Some(java) = bundled_java(&files) {
            if let Some(ver) = java_version(&java) {
                return Ok((java, format!("bundled ({ver})")));
            }
        }
    }
    if let Some((p, ver)) = detect_system_java() {
        return Ok((p, format!("auto-detected ({ver})")));
    }
    Err("No Java runtime found. Reinstall the launcher to restore the bundled Java 8.".into())
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
        let _ = save_dir;
        Ok(files_dir.join("natives"))
    }
}

#[tauri::command]
async fn launch_game(app: AppHandle, tag: String) -> Result<(), String> {
    let files_dir = find_files_dir(&app)
        .ok_or("Could not locate the bundled files/ folder. Please reinstall the launcher.")?;
    let root = data_root(&app);
    let save_dir = root.join("save");
    fs::create_dir_all(&save_dir).map_err(|e| e.to_string())?;

    let alts = save_dir.join("Tenacity").join("Alts.json");
    if !alts.exists() {
        fs::create_dir_all(alts.parent().unwrap()).map_err(|e| e.to_string())?;
        fs::write(&alts, "[]").map_err(|e| format!("Failed to create Alts.json: {e}"))?;
    }

    let (java, java_source) = resolve_java(&app)?;
    let jar = versions_dir(&app).join(&tag).join(JAR);
    if !jar.exists() {
        return Err(format!("Version {tag} is not installed."));
    }

    let natives = native_dir(&files_dir, &save_dir)?;
    let libs = files_dir.join("libs");
    let assets = files_dir.join("assets");

    let sep = if cfg!(target_os = "windows") { ";" } else { ":" };
    let mut classpath = jar.display().to_string();
    if let Ok(entries) = fs::read_dir(&libs) {
        for entry in entries.flatten() {
            if entry.path().extension().is_some_and(|e| e == "jar") {
                classpath.push_str(&format!("{sep}{}", entry.path().display()));
            }
        }
    } else {
        return Err(format!("Could not read libs folder: {}", libs.display()));
    }

    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = Command::new("arch");
        c.arg("-x86_64").arg(&java);
        c
    };
    #[cfg(not(target_os = "macos"))]
    let mut cmd = Command::new(&java);

    #[cfg(target_os = "macos")]
    cmd.arg("-XstartOnFirstThread");

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
        .arg("480")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    let _ = app.emit(
        "game-output",
        GameOutput {
            line: format!("Launching with Java ({java_source}): {}", java.display()),
            kind: "out".into(),
        },
    );

    let child = cmd
        .spawn()
        .map_err(|e| format!("Failed to launch the game: {e}"))?;
    let _ = app.emit("game-launched", tag.clone());

    forward_output(app.clone(), tag.clone(), child);
    Ok(())
}

fn forward_output(app: AppHandle, tag: String, mut child: std::process::Child) {
    use std::io::{BufRead, BufReader};

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    if let Some(out) = stdout {
        let app_io = app.clone();
        tauri::async_runtime::spawn(async move {
            let reader = BufReader::new(out);
            for line in reader.lines() {
                if let Ok(line) = line {
                    let _ = app_io.emit(
                        "game-output",
                        GameOutput {
                            line,
                            kind: "out".into(),
                        },
                    );
                }
            }
        });
    }
    if let Some(err) = stderr {
        let app_io = app.clone();
        tauri::async_runtime::spawn(async move {
            let reader = BufReader::new(err);
            for line in reader.lines() {
                if let Ok(line) = line {
                    let _ = app_io.emit(
                        "game-output",
                        GameOutput {
                            line,
                            kind: "err".into(),
                        },
                    );
                }
            }
        });
    }

    tauri::async_runtime::spawn(async move {
        let code = child.wait().ok().and_then(|s| s.code());
        let _ = app.emit("game-exited", GameExit { tag, code });
    });
}

#[tauri::command]
fn runtime_status(app: AppHandle) -> RuntimeInfo {
    let files_dir = find_files_dir(&app)
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let (path, source, ver) = match resolve_java(&app) {
        Ok((p, source)) => {
            let ver = java_version(&p).unwrap_or_default();
            (p.display().to_string(), source, ver)
        }
        Err(_) => (String::new(), "none".to_string(), String::new()),
    };
    RuntimeInfo {
        files_dir,
        java_path: path,
        java_source: source,
        java_version: ver,
    }
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
            launch_game,
            runtime_status
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}