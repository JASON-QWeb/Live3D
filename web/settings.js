export const el = (id) => document.getElementById(id);
const keyNames = { vidu: "Vidu API Key", llm: "语言模型密钥", youtube: "YouTube Data API Key" };
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
    if (response.status === 401) throw new Error("请从 Avatar3D 应用打开控制台");
    const body = await response.json().catch(() => ({}));
    throw new Error(body.error || "请求失败（HTTP " + response.status + "）");
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
let onUpdate = () => {};

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
    label.textContent = title;
    badge.className = keys[name] ? "key-badge is-set" : "key-badge";
    badge.textContent = keys[name] ? "已配置" : "未配置";
    head.append(label, badge);
    input.placeholder = "保存到项目 .env";
    button.type = "button";
    button.textContent = "保存";
    button.onclick = async () => {
      if (!input.value.trim()) {
        notify("请先填写密钥");
        return;
      }
      try {
        await api("/api/key/" + name, { value: input.value }, "PUT");
        input.value = "";
        await refreshKeys();
        notify("凭据已保存");
      } catch (e) {
        notify(e.message);
      }
    };
    row.className = "keyline";
    row.append(input, button);
    if (keys[name]) {
      const clear = document.createElement("button");
      clear.type = "button";
      clear.textContent = "移除";
      clear.onclick = async () => {
        if (!(await confirmAction("移除凭据", "从项目 .env 文件中移除此密钥。", "移除"))) return;
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
  if (chip) chip.textContent = isEn ? "英文直播 · EN" : "中文直播 · ZH";
}

export async function loadConfig() {
  const d = await api("/api/config");
  configuration = d;
  const form = el("settings");
  for (const [k, v] of Object.entries(d.settings)) {
    const input = form.elements.namedItem(k);
    if (!input) continue;
    if (input.type === "checkbox") input.checked = v;
    else if (Array.isArray(v)) input.value = v.join("\n");
    else input.value = v;
  }
  syncLanguageUI(d.settings.live_language || "zh");
  syncKeys(d.keys);
  el("dataPath").textContent = d.data_dir;
  const imgStatus = el("imageStatus");
  imgStatus.textContent = d.avatar ? "已保存角色图" : "尚未上传角色图";
  imgStatus.classList.toggle("is-live", !!d.avatar);
  el("deleteImage").hidden = !d.avatar;
  onUpdate();
  return d;
}

export async function voiceStatus() {
  try {
    const [v, m] = await Promise.all([api("/api/voice"), api("/api/model")]);
    const vs = el("voiceState");
    const ms = el("modelState");
    vs.textContent = "本机语音 · " + v.message;
    vs.classList.toggle("is-ready", /^已就绪/.test(v.message));
    ms.textContent = "本机对话 · " + m.message;
    ms.classList.toggle("is-ready", /^已就绪/.test(m.message));
  } catch (e) {
    const vs = el("voiceState");
    vs.textContent = e.message;
    vs.classList.remove("is-ready");
  }
}

export function setupConfig(updated) {
  onUpdate = updated;
  el("settings").addEventListener("submit", (e) => e.preventDefault());
  el("liveLanguage")?.addEventListener("change", (e) => syncLanguageUI(e.target.value));
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
        ? "已切换直播语言环境，旧语言语音服务已停止，请点击「启动本机 MLX」加载新语言"
        : "设置已保存",
    );
  });
  el("image").onchange = async (e) => {
    const f = e.target.files[0];
    if (!f) return;
    try {
      await request("/api/avatar", { method: "PUT", body: f });
      configuration.avatar = true;
      const imgStatus = el("imageStatus");
      imgStatus.textContent = "已保存角色图";
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
    if (!(await confirmAction("移除角色图", "只移除 Avatar3D 保存的本机副本。", "移除"))) return;
    await request("/api/avatar", { method: "DELETE" });
    configuration.avatar = false;
    el("deleteImage").hidden = true;
    const imgStatus = el("imageStatus");
    imgStatus.textContent = "尚未上传角色图";
    imgStatus.classList.remove("is-live");
    onUpdate();
  });
  action("startModel", async () => {
    notify((await api("/api/model", {})).message);
    await voiceStatus();
  });
  action("startVoice", async () => {
    notify((await api("/api/voice", {})).message);
    await voiceStatus();
  });
  action("checkVidu", async () => {
    const b = await api("/api/balance");
    const labels = { test: "测试", metered: "计量", concurrent: "并发" };
    el("balance").textContent =
      b.balances.map((n) => (labels[n.type] || n.type) + " " + n.credits + " 积分").join(" · ") ||
      "鉴权通过，尚无资源包";
  });
  action("setupObs", async () => {
    const res = await api("/api/obs", { port: Number(el("obsPort").value), password: el("obsPass").value });
    el("obsPass").value = "";
    el("obsStatus").textContent = (res.created ? "已建立 · " : "已就绪 · ") + res.scene;
  });
  let ingestTimer;
  action("copyIngest", async () => {
    const a = await api("/api/access");
    await navigator.clipboard.writeText(location.origin + "/api/ingest\nAuthorization: Bearer " + a.ingest);
    const btn = el("copyIngest");
    btn.textContent = "已复制弹幕接入信息";
    clearTimeout(ingestTimer);
    ingestTimer = setTimeout(() => {
      btn.textContent = "复制本机弹幕接入信息（抖音/TikTok/YouTube）";
    }, 1800);
    notify("已复制弹幕入口和专用令牌");
  });
  action("quit", async () => {
    if (await confirmAction("退出 Avatar3D", "正在进行的连线会先请求挂断并与云端核对。", "退出")) {
      await api("/api/quit", {});
      notify("正在退出 Avatar3D");
    }
  });
}
