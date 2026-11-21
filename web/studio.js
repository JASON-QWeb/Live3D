import {
  el,
  api,
  request,
  token,
  action,
  notify,
  confirmAction,
  setupConfig,
  loadConfig,
  configuration,
  voiceStatus,
} from "./settings.js";

let mode = "say",
  state,
  avatarUrl = "",
  frameUrl = "",
  timer = 0,
  collecting = false,
  visibleConfig = false;
const names = {
  idle: "待机",
  starting: "连线中",
  live: "已连线",
  reconnecting: "恢复中",
  stopping: "结束中",
  unconfirmed: "待核对",
  ended: "已结束",
  error: "连接失败",
};
const busy = () => state && ["live", "starting", "reconnecting", "stopping"].includes(state.phase);

function setButtonLabel(button, text) {
  const span = button.querySelector("span:last-child");
  if (span) span.textContent = text;
  else button.textContent = text;
}

function updateCharCount() {
  const counter = el("charCount");
  if (counter) counter.textContent = el("line").value.length + " / 600";
}

function tab(id) {
  visibleConfig = id === "config";
  document.querySelectorAll(".tab").forEach((n) => {
    const active = n.dataset.tab === id;
    n.classList.toggle("active", active);
    if (active) n.setAttribute("aria-current", "page");
    else n.removeAttribute("aria-current");
  });
  el("config").hidden = !visibleConfig;
  el("desk").hidden = visibleConfig;
  if (visibleConfig) voiceStatus();
}
document.querySelectorAll(".tab").forEach((n) => (n.onclick = () => tab(n.dataset.tab)));

function timespan(ms) {
  const secs = Math.max(0, Math.floor(ms / 1000));
  return [Math.floor(secs / 3600), Math.floor(secs / 60) % 60, secs % 60]
    .map((n) => String(n).padStart(2, "0"))
    .join(":");
}

function showPoster() {
  const source = frameUrl || avatarUrl;
  if (source && el("poster").src !== source) el("poster").src = source;
  el("poster").hidden = !source;
  el("screenEmpty").hidden = !!source;
  const screenBox = el("screenBox");
  if (screenBox) screenBox.dataset.hasFrame = String(!!source);
  const preview = el("previewType");
  preview.textContent = frameUrl ? "实时监看" : avatarUrl ? "角色参考图" : "未收到画面";
  preview.classList.toggle("is-live", !!frameUrl);
  el("emptyTitle").textContent = configuration?.settings.avatar_id ? "已配置云端形象，等待画面" : "等待角色画面";
  el("chooseRole").hidden = !!configuration?.settings.avatar_id;
}

let avatarRevision = 0;
async function refreshAvatar() {
  const rev = ++avatarRevision;
  try {
    const u = configuration?.avatar ? URL.createObjectURL(await (await request("/api/avatar")).blob()) : "";
    if (rev !== avatarRevision) {
      if (u) URL.revokeObjectURL(u);
      return;
    }
    if (avatarUrl) URL.revokeObjectURL(avatarUrl);
    avatarUrl = u;
    showPoster();
  } catch (e) {
    notify(e.message);
  }
}

function makeEmptyEvents() {
  const box = document.createElement("div");
  box.className = "empty-events";
  const pill = document.createElement("span");
  pill.className = "audit-pill";
  pill.innerHTML = '<i aria-hidden="true"></i>监听就绪';
  const well = document.createElement("span");
  well.className = "empty-well";
  well.setAttribute("aria-hidden", "true");
  well.innerHTML =
    '<svg viewBox="0 0 20 20"><path d="M10 3.2l5.6 2.2v4.2c0 3.5-2.3 6.1-5.6 7-3.3-.9-5.6-3.5-5.6-7V5.4L10 3.2Z" /><path d="M7.8 10.1l1.5 1.5 3-3.1" /></svg>';
  const text = document.createElement("span");
  text.className = "empty-events-text";
  text.textContent = "暂无直播事件";
  box.append(pill, well, text);
  return box;
}

function paintEvents(v) {
  const list = el("events");
  const stamp = JSON.stringify(v.events);
  if (list.dataset.stamp === stamp) return;
  list.dataset.stamp = stamp;
  list.replaceChildren();
  el("eventCount").textContent = v.events.length ? v.events.length + " 条" : "";
  if (!v.events.length) {
    list.append(makeEmptyEvents());
    return;
  }
  for (const e of [...v.events].reverse()) {
    const item = document.createElement("div"),
      time = document.createElement("time"),
      type = document.createElement("small"),
      p = document.createElement("p");
    time.textContent = new Date(e.at).toLocaleTimeString("zh-CN", { hour12: false });
    type.textContent = e.kind;
    p.textContent = e.text;
    item.className = "event";
    item.append(time, type, p);
    list.append(item);
  }
}

function paint(v) {
  state = v;
  const live = v.phase === "live";
  el("phase").textContent = names[v.phase] || v.phase;
  el("lamp").className = live ? "live" : busy() || v.phase === "unconfirmed" ? "warn" : "";
  el("start").hidden = busy();
  el("start").disabled = v.phase === "unconfirmed";
  el("stop").hidden = !busy();
  el("stop").disabled = v.phase === "stopping";
  el("send").disabled = !live;
  el("interrupt").disabled = !live;
  for (const id of ["media", "reference", "reset", "removeRef"]) el(id).disabled = !live;
  el("choreo").disabled = !live || !configuration?.settings.actions.length;
  el("choreo").textContent = v.choreo ? "暂停动作编排" : "开始动作编排";
  el("playTrack").disabled = !live || !el("tracks").value;
  el("soundcheck").disabled = !!busy() || !v.output;
  el("message").textContent = v.message;
  el("outputState").textContent = v.output || "舞台未打开";
  const stageDot = el("stageDot");
  if (stageDot) stageDot.classList.toggle("is-ready", !!v.output);
  el("clock").textContent = v.started ? timespan((v.ended || Date.now()) - v.started) : "00:00:00";
  el("pending").textContent = v.pending + (v.voice_busy ? " · 播出中" : "");
  const strip = el("signalStrip");
  if (strip) {
    strip.dataset.live = String(live);
    strip.dataset.speaking = String(live && !!v.voice_busy);
    const caption = el("signalCaption");
    if (caption) {
      caption.textContent = live
        ? v.voice_busy
          ? "直播中 · 语音播出"
          : "直播中 · 推流正常"
        : busy()
          ? "连线中 · 建立信令"
          : v.output
            ? "待机 · 舞台已连接"
            : "待机 · 本机就绪";
    }
  }
  const m = v.metrics;
  el("latency").textContent =
    m.last_first_audio_ms === null ? "—" : (m.last_first_audio_ms / 1000).toFixed(2) + " 秒";
  el("dimensions").textContent = m.output_width
    ? m.output_width +
      " × " +
      m.output_height +
      (m.output_fps != null ? " · " + m.output_fps.toFixed(1) + " fps" : "")
    : "—";
  el("bill").textContent =
    m.credits != null
      ? m.credits + " 积分"
      : m.billed_seconds != null
        ? m.billed_seconds + " 秒"
        : v.phase === "ended"
          ? "平台未回传金额"
          : "—";
  el("recovery").hidden = v.phase !== "unconfirmed";
  el("recordRow").hidden = !(v.phase === "ended" && configuration?.settings.recording);
  if (!live && frameUrl) {
    URL.revokeObjectURL(frameUrl);
    frameUrl = "";
    showPoster();
  }
  paintEvents(v);
}

async function tick() {
  if (collecting) return;
  collecting = true;
  try {
    const v = await api("/api/status");
    paint(v);
    timer++;
    if (v.phase === "live" && !visibleConfig && timer % 2 === 0) {
      const r = await request("/api/monitor");
      if (frameUrl) URL.revokeObjectURL(frameUrl);
      frameUrl = r.status === 204 ? "" : URL.createObjectURL(await r.blob());
      showPoster();
    }
    if (visibleConfig && timer % 6 === 0) await voiceStatus();
  } catch (e) {
    el("phase").textContent = "本机连接中断";
    el("lamp").className = "warn";
    el("message").textContent = e.message;
    el("send").disabled = true;
    el("start").disabled = true;
  } finally {
    collecting = false;
  }
}

setupConfig(() => {
  refreshAvatar();
  if (state) paint(state);
});

action("chooseRole", () => tab("config"));

action("start", async () => {
  if (!configuration.keys.vidu || (!configuration.avatar && !configuration.settings.avatar_id)) {
    tab("config");
    notify("开播需填写 Vidu 密钥，并上传角色图或填写形象 ID");
    return;
  }
  const min = configuration.settings.max_minutes;
  if (
    !(await confirmAction(
      "开始云端连线",
      "当前时限 " + min + " 分钟。按组件版公开价折合约 ¥" + (min * 1.875).toFixed(2) + "，实际结算以 Vidu 为准。",
      "开始连线",
    ))
  )
    return;
  await api("/api/start", {});
  await tick();
});

action("stop", async () => {
  await api("/api/stop", {});
  await tick();
});

action("recover", async () => {
  await api("/api/recover", {});
  await tick();
});

action("ack", async () => {
  if (
    await confirmAction(
      "确认云端已结束",
      "仅在 Vidu 控制台确认此场次已经结束后使用。Avatar3D 会保留人工核对记录。",
      "确认已结束",
    )
  ) {
    await api("/api/recover", { operator_confirmed: true });
    await tick();
  }
});

async function stageUrl() {
  return location.origin + "/stage#" + (await api("/api/access")).stage;
}

let copyTimer;
action("copyStage", async () => {
  await navigator.clipboard.writeText(await stageUrl());
  const btn = el("copyStage");
  btn.classList.add("is-copied");
  setButtonLabel(btn, "已复制地址");
  clearTimeout(copyTimer);
  copyTimer = setTimeout(() => {
    btn.classList.remove("is-copied");
    setButtonLabel(btn, "复制 OBS 地址");
  }, 1800);
  notify("已复制 OBS 浏览器源地址");
});

action("openStage", async () => {
  const w = window.open("about:blank", "avatar3d_stage");
  if (!w) throw Error("请允许浏览器打开舞台窗口");
  w.opener = null;
  w.location.replace(await stageUrl());
});

el("modes").onclick = (e) => {
  const b = e.target.closest("button");
  if (!b) return;
  mode = b.dataset.mode;
  el("modes")
    .querySelectorAll("button")
    .forEach((n) => n.setAttribute("aria-pressed", String(b === n)));
  el("line").placeholder = { say: "输入要说的话", reply: "输入给角色的问题", action: "输入角色要执行的动作" }[
    mode
  ];
  el("send").textContent = mode === "action" ? "发送动作" : "加入播出";
  el("soundcheck").hidden = mode !== "say";
  el("audition").hidden = mode === "action";
  setButtonLabel(el("audition"), mode === "reply" ? "试听回复" : "本机试听");
};

el("line").addEventListener("input", updateCharCount);

action("send", async () => {
  const text = el("line").value.trim();
  if (!text) throw Error("请输入播出内容");
  await api("/api/command", { kind: mode, text });
  el("line").value = "";
  updateCharCount();
  await tick();
});

el("line").onkeydown = (e) => {
  if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
    e.preventDefault();
    el("send").click();
  }
};

action("interrupt", async () => {
  await api("/api/command", { kind: "interrupt" });
  await tick();
});

action("soundcheck", async () => {
  const text = el("line").value.trim();
  if (!text) throw Error("请输入试音文字");
  await api("/api/soundcheck", { text });
  notify("已向独立舞台发送试音");
  await tick();
});

let audioUrl = "";
action("audition", async () => {
  const text = el("line").value.trim();
  if (!text) throw Error("请输入要试听的文字");
  setButtonLabel(el("audition"), "合成中…");
  try {
    const r = await request(mode === "reply" ? "/api/preview/reply" : "/api/preview", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ text }),
    });
    if (audioUrl) URL.revokeObjectURL(audioUrl);
    audioUrl = URL.createObjectURL(await r.blob());
    const player = el("audio");
    player.src = audioUrl;
    player.hidden = false;
    await player.play();
  } finally {
    setButtonLabel(el("audition"), mode === "reply" ? "试听回复" : "本机试听");
  }
});

async function fileEvent(input, path) {
  const file = input.files[0];
  if (!file) return;
  try {
    await request(path, { method: "POST", body: file });
    notify("已提交");
    await tick();
  } catch (e) {
    notify(e.message);
  } finally {
    input.value = "";
  }
}
el("media").onchange = (e) => fileEvent(e.target, "/api/media");
el("reference").onchange = (e) => fileEvent(e.target, "/api/reference/" + el("refKind").value);

action("choreo", async () => {
  await api("/api/choreo", { on: !state.choreo });
  await tick();
});

async function loadTracks() {
  const items = await api("/api/tracks"),
    select = el("tracks"),
    old = select.value;
  select.replaceChildren();
  if (!items.length) select.add(new Option("曲库为空", ""));
  for (const t of items) {
    select.add(new Option(t.title + " · " + timespan(t.duration_ms), t.id));
  }
  if (items.some((t) => t.id === old)) select.value = old;
  el("removeTrack").disabled = !items.length;
  el("playTrack").disabled = !items.length || state?.phase !== "live";
}

el("addTrack").onchange = async (e) => {
  const f = e.target.files[0];
  if (!f) return;
  try {
    const title = f.name.replace(/\.[^.]+$/, "");
    await request("/api/tracks/" + encodeURIComponent(title), { method: "PUT", body: f });
    await loadTracks();
    notify("已导入曲库");
  } catch (e) {
    notify(e.message);
  } finally {
    el("addTrack").value = "";
  }
};

action("playTrack", async () => {
  await api("/api/tracks/" + el("tracks").value, {});
  notify("曲目已加入播出队列");
});

action("removeTrack", async () => {
  if (await confirmAction("从曲库移除", "移除所选曲目在 Avatar3D 中的副本。", "移除")) {
    await request("/api/tracks/" + el("tracks").value, { method: "DELETE" });
    await loadTracks();
  }
});

action("reset", () => api("/api/command", { kind: "reset" }));
action("removeRef", () => api("/api/command", { kind: "remove_reference" }));

action("record", async () => {
  const v = await api("/api/recording", { id: state.live_id });
  const urls = [];
  function collect(x) {
    if (typeof x === "string" && x.startsWith("https://")) urls.push(x);
    else if (x && typeof x === "object") Object.values(x).forEach(collect);
  }
  collect(v);
  el("recordLinks").replaceChildren();
  if (!urls.length) {
    notify("云端录播尚未返回下载地址");
    return;
  }
  urls.forEach((u, i) => {
    const a = document.createElement("a");
    a.href = u;
    a.target = "_blank";
    a.rel = "noreferrer";
    a.textContent = "下载录播 " + (i + 1);
    el("recordLinks").append(a);
  });
});

updateCharCount();

if (!token) {
  notify("请从 Avatar3D 应用打开控制台");
  el("phase").textContent = "需要本机授权";
  el("lamp").className = "warn";
  el("start").disabled = true;
} else {
  try {
    await loadConfig();
    await loadTracks();
    await tick();
    setInterval(tick, 1200);
  } catch (e) {
    notify(e.message);
  }
}
