import { t, tr } from "./i18n.js";

export const el = (id) => document.getElementById(id);
const keyNames = {
  vidu: "Vidu API Key",
  llm: "语言模型密钥",
  youtube: "YouTube Data API Key",
  douyin: "抖音 Cookie（选填，匿名连接受限时用）",
};
const incoming = location.hash.slice(1);
if (incoming) {
  sessionStorage.setItem("avatar3d.admin", incoming);
  history.replaceState(null, "", location.pathname);
}
export const token = sessionStorage.getItem("avatar3d.admin") || "";

export async function request(path, options = {}) {
  const headers = new Headers(options.headers);
  headers.set("Authorization", "Bearer " + token);
  const response = await fetch(path, { ...options, headers, cache: "no-store" });
  if (!response.ok) {
    if (response.status === 401) throw new Error(t("请从 Avatar3D 应用打开控制台"));
    const body = await response.json().catch(() => ({}));
    // raw keeps the backend's Chinese text for code that matches on it.
    const error = new Error(body.error ? tr(body.error) : t("请求失败（HTTP {0}）", response.status));
    error.raw = body.error || "";
    throw error;
  }
  return response;
}
export async function api(path, payload, method) {
  const opts = { method: method || (payload === undefined ? "GET" : "POST") };
  if (payload !== undefined) {
    opts.headers = { "Content-Type": "application/json" };
    opts.body = JSON.stringify(payload);
  }
  return (await request(path, opts)).json();
}
let timer;
export function notify(text) {
  el("notice").textContent = text;
  el("notice").hidden = false;
  clearTimeout(timer);
  timer = setTimeout(() => {
    el("notice").hidden = true;
  }, 6500);
}
export function action(id, handler) {
  el(id).addEventListener("click", async (event) => {
    event.preventDefault();
    const node = event.currentTarget;
    if (node.dataset.working) return;
    node.dataset.working = "1";
    node.setAttribute("aria-busy", "true");
    try {
      await handler();
    } catch (e) {
      notify(e.message);
    } finally {
      delete node.dataset.working;
      node.removeAttribute("aria-busy");
    }
  });
}
export function confirmAction(title, desc, verb) {
  const dialog = el("confirm");
  if (dialog.open) return Promise.resolve(false);
  el("confirmTitle").textContent = title;
  el("confirmDesc").textContent = desc;
  el("yes").textContent = verb;
  return new Promise((resolve) => {
    dialog.addEventListener("close", () => resolve(dialog.returnValue === "yes"), { once: true });
    el("yes").onclick = () => dialog.close("yes");
    el("no").onclick = () => dialog.close("no");
    dialog.returnValue = "";
    dialog.showModal();
    el("no").focus();
  });
}
export let configuration;
let dirty = false;
/** The settings form has edits that were not saved. */
export const settingsDirty = () => dirty;
let onUpdate = () => {};
const promptDocs = new Map();
const promptErrors = new Map();

function syncKeys(keys) {
  el("keys").replaceChildren();
  for (const [name, title] of Object.entries(keyNames)) {
    const wrap = document.createElement("div"),
      head = document.createElement("div"),
      label = document.createElement("label"),
      badge = document.createElement("span"),
      row = document.createElement("div"),
      input = document.createElement("input"),
      button = document.createElement("button");
    wrap.className = "key-item";
    head.className = "key-head";
    input.type = "password";
    input.autocomplete = "new-password";
    input.id = "key-" + name;
    label.htmlFor = input.id;
    label.textContent = t(title);
    badge.className = keys[name] ? "key-badge is-set" : "key-badge";
    badge.textContent = keys[name] ? t("已配置") : t("未配置");
    head.append(label, badge);
    input.placeholder = t("保存到项目 .env");
    button.type = "button";
    button.textContent = t("保存");
    button.onclick = async () => {
      if (!input.value.trim()) {
        notify(t("请先填写密钥"));
        return;
      }
      try {
        await api("/api/key/" + name, { value: input.value }, "PUT");
        input.value = "";
        await refreshKeys();
        notify(t("凭据已保存"));
      } catch (e) {
        notify(e.message);
      }
    };
    row.className = "keyline";
    row.append(input, button);
    if (keys[name]) {
      const clear = document.createElement("button");
      clear.type = "button";
      clear.textContent = t("移除");
      clear.onclick = async () => {
        if (!(await confirmAction(t("移除凭据"), t("从项目 .env 文件中移除此密钥。"), t("移除")))) return;
        try {
          await api("/api/key/" + name, { value: "" }, "PUT");
          await refreshKeys();
        } catch (e) {
          notify(e.message);
        }
      };
      row.append(clear);
    }
    wrap.append(head, row);
    el("keys").append(wrap);
  }
}

async function refreshKeys() {
  const d = await api("/api/config");
  configuration.keys = d.keys;
  syncKeys(d.keys);
  onUpdate();
}

function syncLanguageUI(lang) {
  const isEn = lang === "en";
  document.querySelectorAll(".lang-zh").forEach((n) => (n.hidden = isEn));
  document.querySelectorAll(".lang-en").forEach((n) => (n.hidden = !isEn));
  const chip = el("langChip");
  if (chip) chip.textContent = isEn ? t("英文直播 · EN") : t("中文直播 · ZH");
  showPrompt(lang);
}

// Shows only the comment-source fields for the chosen platform.
function syncPlatformUI(platform) {
  document.querySelectorAll("[data-platform]").forEach((n) => (n.hidden = n.dataset.platform !== platform));
}

function promptPath(id, lang) {
  return "/api/prompts/" + encodeURIComponent(id) + "/" + lang;
}

function showPrompt(lang = el("liveLanguage").value) {
  const doc = promptDocs.get(lang);
  el("savePrompt").disabled = !doc;
  el("promptSource").textContent = [
    promptErrors.get(lang),
    doc?.path,
    doc?.knowledge_path,
    doc?.active_version && t("生效版本 {0}", doc.active_version),
    tr(doc?.error),
  ].filter(Boolean).join("\n");
}

async function loadPrompt(lang, id = configuration.settings.prompt_profile) {
  const field = el("prompt-" + lang);
  try {
    const doc = await api(promptPath(id, lang));
    if (configuration.settings.prompt_profile !== id) return;
    promptDocs.set(lang, doc);
    promptErrors.delete(lang);
    field.value = doc.text;
    field.disabled = false;
  } catch (e) {
    if (configuration.settings.prompt_profile !== id) return;
    promptErrors.set(lang, e.message);
  }
  showPrompt();
}

export async function loadConfig() {
  const d = await api("/api/config");
  const profileChanged = d.settings.prompt_profile !== configuration?.settings.prompt_profile;
  configuration = d;
  dirty = false;
  const form = el("settings");
  for (const [k, v] of Object.entries(d.settings)) {
    const input = form.elements.namedItem(k);
    if (!input) continue;
    if (input.type === "checkbox") input.checked = v;
    else if (Array.isArray(v)) input.value = v.join("\n");
    else input.value = v;
  }
  syncLanguageUI(d.settings.live_language || "zh");
  syncPlatformUI(d.settings.live_platform);
  if (profileChanged) {
    promptDocs.clear();
    promptErrors.clear();
    for (const lang of ["zh", "en"]) {
      const field = el("prompt-" + lang);
      field.value = "";
      field.disabled = true;
      void loadPrompt(lang);
    }
    showPrompt();
  }
  syncKeys(d.keys);
  el("obsPass").placeholder = d.keys.obs ? t(savedPassHint) : t("未设密码可留空");
  el("dataPath").textContent = d.data_dir;
  const imgStatus = el("imageStatus");
  imgStatus.textContent = d.avatar ? t("已保存角色图") : t("尚未上传角色图");
  imgStatus.classList.toggle("is-live", !!d.avatar);
  el("deleteImage").hidden = !d.avatar;
  onUpdate();
  return d;
}

const startupSteps = {
  env: [1, "准备 Python 环境"],
  packages: [2, "下载运行依赖"],
  download: [3, "下载模型权重"],
  load: [4, "加载模型"],
};

function megabytes(bytes) {
  const mb = bytes / 1048576;
  return mb >= 1024 ? (mb / 1024).toFixed(2) + " GB" : mb.toFixed(mb >= 100 ? 0 : 1) + " MB";
}

// Shows the startup step and download size under a service banner.
function showProgress(prefix, progress) {
  const box = el(prefix + "Progress");
  box.hidden = !progress;
  if (!progress) return;
  const [step, label] = startupSteps[progress.stage];
  const meter = box.querySelector(".meter");
  const known = progress.total > 0;
  const ratio = known ? Math.min(1, progress.done / progress.total) : 0;
  el(prefix + "ProgressStep").textContent = t("第 {0}/4 步 · {1}", step, t(label));
  el(prefix + "ProgressValue").textContent = known
    ? megabytes(progress.done) + " / " + megabytes(progress.total) + " · " + Math.floor(ratio * 100) + "%"
    : progress.done > 0
      ? t("已下载 {0}", megabytes(progress.done))
      : "";
  meter.classList.toggle("is-indeterminate", !known);
  meter.firstElementChild.style.width = known ? (ratio * 100).toFixed(1) + "%" : "";
  if (known) meter.setAttribute("aria-valuenow", String(Math.floor(ratio * 100)));
  else meter.removeAttribute("aria-valuenow");
}

// True while a local service is starting, so the caller can poll faster.
export let servicesStarting = false;

export async function voiceStatus() {
  try {
    const [v, m] = await Promise.all([api("/api/voice"), api("/api/model")]);
    const vs = el("voiceState");
    const ms = el("modelState");
    vs.textContent = t("本机语音 · {0}", tr(v.message));
    vs.classList.toggle("is-ready", /^已就绪/.test(v.message));
    ms.textContent = t("本机对话 · {0}", tr(m.message));
    ms.classList.toggle("is-ready", /^已就绪/.test(m.message));
    showProgress("voice", v.progress);
    showProgress("model", m.progress);
    servicesStarting = v.message === "启动中" || m.message === "启动中";
  } catch (e) {
    const vs = el("voiceState");
    vs.textContent = e.message;
    vs.classList.remove("is-ready");
    servicesStarting = false;
  }
}

const savedPassHint = "已保存，留空沿用";
let stream = null,
  streamPolling = false,
  filledServer = "";

// Port and password typed in the OBS panel. An empty password uses the saved one.
function obsTarget() {
  return { port: Number(el("obsPort").value), password: el("obsPass").value };
}

function obsSent(target) {
  if (!target.password) return;
  el("obsPass").value = "";
  el("obsPass").placeholder = t(savedPassHint);
}

function clock(ms) {
  const secs = Math.floor(ms / 1000);
  return [Math.floor(secs / 3600), Math.floor(secs / 60) % 60, secs % 60]
    .map((n) => String(n).padStart(2, "0"))
    .join(":");
}

function showStream(s, error) {
  stream = s;
  const banner = el("streamState"),
    button = el("obsLive");
  button.disabled = false;
  if (!s) {
    banner.textContent = error;
    banner.classList.remove("is-ready");
    return;
  }
  // Fill the server once, and again only while the operator has not edited it.
  const server = el("streamServer");
  if (s.custom && server.value === filledServer) server.value = filledServer = s.server;
  el("streamKey").placeholder = s.key_set ? t("OBS 已保存推流码，留空则不改") : t("直播平台给的推流码");
  const scene = s.scene ? t("播出场景「{0}」", s.scene) : "";
  let parts;
  if (s.live) {
    const dropped = s.total_frames ? t("掉帧 {0}%", ((100 * s.skipped_frames) / s.total_frames).toFixed(1)) : "";
    parts = [s.reconnecting ? t("正在重连直播平台") : t("直播中 {0}", clock(s.duration_ms)), dropped, scene];
  } else {
    const where = s.custom ? s.server : s.service && t("OBS 内置服务「{0}」", s.service);
    parts = [s.key_set ? t("未开播") : t("未开播 · OBS 还没有推流码"), where, scene];
  }
  banner.textContent = parts.filter(Boolean).join(" · ");
  banner.classList.toggle("is-ready", s.live && !s.reconnecting);
  button.textContent = s.live ? t("结束直播") : t("开始直播");
  button.className = s.live ? "danger" : "primary";
}

// Polls OBS with the saved port and password.
export async function obsStream() {
  if (streamPolling) return;
  streamPolling = true;
  try {
    showStream(await api("/api/obs/stream"));
  } catch (e) {
    showStream(null, e.message);
  } finally {
    streamPolling = false;
  }
}

async function toggleLive() {
  const want = !stream?.live;
  const target = obsTarget();
  const now = await api("/api/obs/stream", target);
  obsSent(target);
  showStream(now);
  if (now.live === want) {
    notify(want ? t("OBS 已经在直播") : t("OBS 没有在直播"));
    return;
  }
  const phase = (await api("/api/status")).phase;
  if (want) {
    const where = now.custom ? now.server : now.service ? t("OBS 内置服务「{0}」", now.service) : t("直播平台");
    const notes = [
      t("OBS 会把{0}推到 {1}，观众马上就能看到。", now.scene ? t("场景「{0}」", now.scene) : t("当前画面"), where),
      !now.avatar_scene && t("当前播出的不是「Avatar3D · 数字人」场景。"),
      phase !== "live" && t("数字人还没有连线，观众会先看到空白舞台。"),
    ];
    if (!(await confirmAction(t("开始直播"), notes.filter(Boolean).join("").trim(), t("开播")))) return;
  } else {
    const notes = [
      t("OBS 会停止推流，直播间随即下播。"),
      phase === "live" && t("数字人连线还在计费，下播后记得在直播台点「结束连线」。"),
    ];
    if (!(await confirmAction(t("结束直播"), notes.filter(Boolean).join("").trim(), t("下播")))) return;
  }
  const res = await api("/api/obs/stream", { ...target, password: "", live: want });
  showStream(res);
  if (res.live === want) notify(want ? t("已开播") : t("已下播"));
  else if (want) notify(t("已让 OBS 开始推流，正在连接直播平台。若一直没有变成「直播中」，请看 OBS 里的错误提示"));
  else notify(t("OBS 正在停止推流"));
}

export async function obsApp() {
  try {
    el("obsMissing").hidden = (await api("/api/obs/app")).installed;
  } catch {
    // Detection is a hint only; the open button still works without it.
  }
}

export function setupConfig(updated) {
  onUpdate = updated;
  el("settings").addEventListener("submit", (e) => e.preventDefault());
  el("settings").addEventListener("input", () => (dirty = true));
  el("liveLanguage")?.addEventListener("change", (e) => syncLanguageUI(e.target.value));
  el("livePlatform").addEventListener("change", (e) => syncPlatformUI(e.target.value));
  action("reloadPrompt", () => loadPrompt(el("liveLanguage").value));
  action("savePrompt", async () => {
    const lang = el("liveLanguage").value;
    const id = configuration.settings.prompt_profile;
    const doc = promptDocs.get(lang);
    if (!doc) return;
    const field = el("prompt-" + lang);
    const text = field.value;
    const saved = await api(promptPath(id, lang), { revision: doc.revision, text }, "PUT");
    if (configuration.settings.prompt_profile !== id) return;
    promptDocs.set(lang, saved);
    promptErrors.delete(lang);
    if (field.value === text) field.value = saved.text;
    showPrompt();
    notify(t("话术已保存"));
  });
  action("save", async () => {
    const form = el("settings");
    if (!form.reportValidity()) return;
    const prevLang = configuration.settings.live_language;
    const c = { ...configuration.settings };
    for (const k of Object.keys(c)) {
      const field = form.elements.namedItem(k);
      if (!field) continue;
      if (Array.isArray(c[k]))
        c[k] = field.value
          .split(/\r?\n/)
          .map((s) => s.trim())
          .filter(Boolean);
      else if (field.type === "checkbox") c[k] = field.checked;
      else if (field.type === "number") c[k] = Number(field.value);
      else c[k] = field.value.trim();
    }
    await api("/api/config", c, "PUT");
    await loadConfig();
    await voiceStatus();
    notify(
      prevLang !== c.live_language
        ? t("已切换直播语言环境，旧语言语音服务已停止，请点击「启动本机 MLX」加载新语言")
        : t("设置已保存"),
    );
  });
  el("image").onchange = async (e) => {
    const f = e.target.files[0];
    if (!f) return;
    try {
      await request("/api/avatar", { method: "PUT", body: f });
      configuration.avatar = true;
      const imgStatus = el("imageStatus");
      imgStatus.textContent = t("已保存角色图");
      imgStatus.classList.add("is-live");
      el("deleteImage").hidden = false;
      onUpdate();
    } catch (e) {
      notify(e.message);
    } finally {
      el("image").value = "";
    }
  };
  action("deleteImage", async () => {
    if (!(await confirmAction(t("移除角色图"), t("只移除 Avatar3D 保存的本机副本。"), t("移除")))) return;
    await request("/api/avatar", { method: "DELETE" });
    configuration.avatar = false;
    el("deleteImage").hidden = true;
    const imgStatus = el("imageStatus");
    imgStatus.textContent = t("尚未上传角色图");
    imgStatus.classList.remove("is-live");
    onUpdate();
  });
  action("startModel", async () => {
    notify(tr((await api("/api/model", {})).message));
    await voiceStatus();
  });
  action("startVoice", async () => {
    notify(tr((await api("/api/voice", {})).message));
    await voiceStatus();
  });
  action("checkVidu", async () => {
    const b = await api("/api/balance");
    const labels = { test: "测试", metered: "计量", concurrent: "并发" };
    el("balance").textContent =
      b.balances.map((n) => t("{0} {1} 积分", labels[n.type] ? t(labels[n.type]) : n.type, n.credits)).join(" · ") ||
      t("鉴权通过，尚无资源包");
  });
  action("openObs", async () => {
    try {
      await api("/api/obs/app", {});
    } catch (e) {
      el("obsMissing").hidden = false;
      throw e;
    }
    el("obsMissing").hidden = true;
    el("obsStatus").textContent = t("已打开 OBS");
    notify(t("已打开 OBS。首次使用请在 OBS「工具 → WebSocket 服务器设置」启用服务器，再点「建立 Avatar3D 场景」"));
  });
  action("setupObs", async () => {
    const target = obsTarget();
    const res = await api("/api/obs", target);
    obsSent(target);
    el("obsStatus").textContent = res.created ? t("已建立 · {0}", res.scene) : t("已就绪 · {0}", res.scene);
    void obsStream();
  });
  action("saveStream", async () => {
    const target = obsTarget();
    const res = await api(
      "/api/obs/stream",
      { ...target, server: el("streamServer").value.trim(), key: el("streamKey").value },
      "PUT",
    );
    obsSent(target);
    el("streamKey").value = "";
    el("streamServer").value = filledServer = res.server;
    showStream(res);
    notify(t("推流地址和推流码已保存到 OBS"));
  });
  action("obsLive", toggleLive);
  let ingestTimer;
  action("copyIngest", async () => {
    const a = await api("/api/access");
    await navigator.clipboard.writeText(location.origin + "/api/ingest\nAuthorization: Bearer " + a.ingest);
    const btn = el("copyIngest");
    btn.textContent = t("已复制弹幕接入信息");
    clearTimeout(ingestTimer);
    ingestTimer = setTimeout(() => {
      btn.textContent = t("复制本机弹幕接入信息（抖音/TikTok/YouTube）");
    }, 1800);
    notify(t("已复制弹幕入口和专用令牌"));
  });
  action("quit", async () => {
    if (await confirmAction(t("退出 Avatar3D"), t("正在进行的连线会先请求挂断并与云端核对。"), t("退出"))) {
      await api("/api/quit", {});
      notify(t("正在退出 Avatar3D"));
    }
  });
}
