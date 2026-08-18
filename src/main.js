import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";

const DISCORD_INVITE = "https://discord.gg/BrF6sfsaBp";

const state = {
  installed: new Map(),
  releases: [],
  selected: null,
  downloading: new Set(),
};

const $ = (sel) => document.querySelector(sel);

const installedList = $("#installed-list");
const installedEmpty = $("#installed-empty");
const availableList = $("#available-list");
const availableEmpty = $("#available-empty");
const selectedTag = $("#selected-tag");
const launchBtn = $("#launch-btn");
const launchStatus = $("#launch-status");
const consoleBox = $("#console");

function log(message, kind = "") {
  const line = document.createElement("div");
  line.className = `line ${kind}`;
  const ts = new Date().toLocaleTimeString([], { hour12: false });
  line.innerHTML = `<span class="ts">[${ts}]</span>${message}`;
  consoleBox.appendChild(line);
  consoleBox.scrollTop = consoleBox.scrollHeight;
}

function formatSize(bytes) {
  if (bytes >= 1073741824) return (bytes / 1073741824).toFixed(2) + " GB";
  if (bytes >= 1048576) return (bytes / 1048576).toFixed(1) + " MB";
  return Math.max(1, Math.round(bytes / 1024)) + " KB";
}

function formatDate(iso) {
  if (!iso) return "unknown date";
  return new Date(iso).toLocaleDateString([], {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

function selectVersion(tag) {
  state.selected = tag;
  selectedTag.textContent = tag;
  launchBtn.disabled = false;
  launchStatus.textContent = "";
  launchStatus.className = "launch-status";
  document.querySelectorAll(".version-item").forEach((el) => {
    el.classList.toggle("selected", el.dataset.tag === tag);
  });
}

function render() {
  renderInstalled();
  renderAvailable();
}

function renderInstalled() {
  installedList.innerHTML = "";
  const tags = [...state.installed.keys()];
  installedEmpty.style.display = tags.length ? "none" : "block";

  for (const tag of tags) {
    const v = state.installed.get(tag);
    const item = document.createElement("div");
    item.className = "version-item" + (state.selected === tag ? " selected" : "");
    item.dataset.tag = tag;

    const info = document.createElement("div");
    info.className = "version-info";
    info.innerHTML = `<span class="version-tag"></span><span class="version-meta">${formatSize(v.size)} · installed locally</span>`;
    info.querySelector(".version-tag").textContent = tag;
    item.appendChild(info);

    const actions = document.createElement("div");
    actions.className = "version-actions";

    const play = document.createElement("button");
    play.className = "btn-mini btn-play-mini";
    play.innerHTML = `<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M8 5.14v13.72a1 1 0 0 0 1.5.86l11-6.86a1 1 0 0 0 0-1.72l-11-6.86a1 1 0 0 0-1.5.86z"></path></svg>Play`;
    play.addEventListener("click", () => selectVersion(tag));
    actions.appendChild(play);

    const del = document.createElement("button");
    del.className = "btn-mini btn-delete";
    del.innerHTML = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3 6h18M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2m3 0v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6h14z"></path></svg>Delete`;
    del.addEventListener("click", () => deleteVersion(tag));
    actions.appendChild(del);

    item.appendChild(actions);
    installedList.appendChild(item);
  }
}

function renderAvailable() {
  availableList.innerHTML = "";
  const list = state.releases.filter((r) => r.asset);
  availableEmpty.style.display = list.length ? "none" : "block";
  availableEmpty.innerHTML = list.length
    ? availableEmpty.innerHTML
    : `<p>No releases with a Tenacity.jar found.</p>`;

  for (const rel of list) {
    const installed = state.installed.has(rel.tag);
    const downloading = state.downloading.has(rel.tag);
    const isLatest = state.releases.find((r) => r.asset)?.tag === rel.tag;

    const item = document.createElement("div");
    item.className = "version-item";
    item.dataset.tag = rel.tag;

    const info = document.createElement("div");
    info.className = "version-info";
    const latestBadge = isLatest ? `<span class="badge-latest">LATEST</span>` : "";
    info.innerHTML = `<span class="version-tag">${rel.tag}${latestBadge}</span>`;
    const meta = document.createElement("span");
    meta.className = "version-meta";
    meta.textContent = `${formatDate(rel.published_at)} · ${formatSize(rel.asset.size)}`;
    info.appendChild(meta);
    item.appendChild(info);

    const actions = document.createElement("div");
    actions.className = "version-actions";

    if (installed || downloading) {
      const badge = document.createElement("span");
      badge.className = "installed-badge";
      badge.textContent = downloading ? "Downloading…" : "Installed";
      actions.appendChild(badge);
    } else {
      const install = document.createElement("button");
      install.className = "btn-mini btn-install";
      install.innerHTML = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M7 10l5 5 5-5M12 15V3"></path></svg>Install`;
      install.addEventListener("click", () => installVersion(rel.tag));
      actions.appendChild(install);
    }

    item.appendChild(actions);
    availableList.appendChild(item);

    if (downloading) {
      const bar = document.createElement("div");
      bar.className = "download-progress";
      const fill = document.createElement("div");
      fill.className = "download-progress-bar";
      bar.appendChild(fill);
      item.appendChild(bar);
      item.dataset.progressTag = rel.tag;
    }
  }
}

function updateProgress(tag, downloaded, total) {
  const item = document.querySelector(`[data-progress-tag="${tag}"]`);
  if (!item) return;
  const fill = item.querySelector(".download-progress-bar");
  if (!fill) return;
  const pct = total > 0 ? Math.min(100, Math.round((downloaded / total) * 100)) : 0;
  fill.style.width = pct + "%";
  const badge = item.querySelector(".installed-badge");
  if (badge) badge.textContent = `${pct}% · ${formatSize(downloaded)}`;
}

async function loadInstalled() {
  try {
    const list = await invoke("list_installed");
    state.installed = new Map(list.map((v) => [v.tag, v]));
    if (state.selected && !state.installed.has(state.selected)) {
      state.selected = null;
      selectedTag.textContent = "—";
      launchBtn.disabled = true;
    }
    renderInstalled();
  } catch (err) {
    log(`Failed to load installed versions: ${err}`, "err");
  }
}

async function loadReleases() {
  $("#refresh-btn").classList.add("spinning");
  availableEmpty.style.display = "block";
  availableEmpty.innerHTML = "<p>Fetching releases from GitHub…</p>";
  try {
    const releases = await invoke("list_releases");
    state.releases = releases;
    renderAvailable();
    log(`Fetched ${releases.length} releases from GitHub.`, "ok");
  } catch (err) {
    availableEmpty.style.display = "block";
    availableEmpty.innerHTML = `<p>Could not reach GitHub.</p><p class="empty-hint">${err}</p>`;
    log(`Failed to fetch releases: ${err}`, "err");
  } finally {
    $("#refresh-btn").classList.remove("spinning");
  }
}

async function installVersion(tag) {
  state.downloading.add(tag);
  renderAvailable();
  log(`Downloading ${tag}…`);
  try {
    await invoke("install_version", { tag });
    log(`${tag} installed.`, "ok");
  } catch (err) {
    log(`Failed to install ${tag}: ${err}`, "err");
  } finally {
    state.downloading.delete(tag);
    await loadInstalled();
    renderAvailable();
  }
}

async function deleteVersion(tag) {
  try {
    await invoke("delete_version", { tag });
    log(`Deleted ${tag}.`, "ok");
    if (state.selected === tag) {
      state.selected = null;
      selectedTag.textContent = "—";
      launchBtn.disabled = true;
    }
    await loadInstalled();
    renderAvailable();
  } catch (err) {
    log(`Failed to delete ${tag}: ${err}`, "err");
  }
}

async function launchGame() {
  if (!state.selected) return;
  launchBtn.disabled = true;
  launchStatus.className = "launch-status";
  launchStatus.textContent = "Launching…";
  log(`Launching Tenacity ${state.selected}…`);
  try {
    await invoke("launch_game", { tag: state.selected });
    launchStatus.className = "launch-status ok";
    launchStatus.textContent = "Game launched";
    log(`Game process started (${state.selected}).`, "ok");
  } catch (err) {
    launchStatus.className = "launch-status err";
    launchStatus.textContent = "Launch failed";
    log(`Launch failed: ${err}`, "err");
  } finally {
    launchBtn.disabled = !state.selected;
  }
}

function setupTheme() {
  const saved = localStorage.getItem("theme");
  if (saved) document.documentElement.dataset.theme = saved;
  $("#theme-toggle").addEventListener("click", () => {
    const next = document.documentElement.dataset.theme === "dark" ? "light" : "dark";
    document.documentElement.dataset.theme = next;
    localStorage.setItem("theme", next);
    log(`Theme switched to ${next} mode.`);
  });
}

function setupDiscord() {
  $("#discord-btn").addEventListener("click", async () => {
    try {
      await openUrl(DISCORD_INVITE);
    } catch (err) {
      log(`Could not open Discord: ${err}`, "err");
    }
  });
}

async function init() {
  setupTheme();
  setupDiscord();

  log("Tenacity Launcher v2.0.0");
  log("Loading local versions…");
  await loadInstalled();
  await loadReleases();
  if (state.selected) selectVersion(state.selected);

  listen("download-progress", (event) => {
    const { tag, downloaded, total } = event.payload;
    updateProgress(tag, downloaded, total);
  });

  listen("versions-changed", () => loadInstalled());

  listen("game-output", (event) => {
    const { line, kind } = event.payload;
    log(line, kind === "err" ? "err" : "");
  });

  listen("game-exited", (event) => {
    const { tag, code } = event.payload;
    log(`Tenacity ${tag} exited with code ${code ?? "?"}.`, code === 0 ? "ok" : "err");
    launchBtn.disabled = !state.selected;
    launchStatus.className = "launch-status";
    launchStatus.textContent = "Game closed";
  });

  launchBtn.addEventListener("click", launchGame);
}

init();