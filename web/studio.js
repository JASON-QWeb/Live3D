import { loadDances, paintDance } from "./studio-dance.js";
import { setupOnlineBgm, paintOnlineBgm, loadBgm as loadBgmList } from "./online-bgm.js";
import { t, tr, lang, setLang } from "./i18n.js";
import {
  el,
  api,
  copyText,
  request,
  token,
  action,
  notify,
  confirmAction,
  setupConfig,
  loadConfig,
  configuration,
  voiceStatus,
  servicesStarting,
  obsApp,
  obsStream,
  settingsDirty,
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
  if (visibleConfig) {
    voiceStatus();
    obsApp();
    obsStream();
  }
}
document.querySelectorAll(".tab").forEach((n) => (n.onclick = () => tab(n.dataset.tab)));

const langButton = el("uiLang");
langButton.textContent = lang === "en" ? "中文" : "English";
langButton.onclick = async () => {
  if (
    settingsDirty() &&
    !(await confirmAction(
      t("切换界面语言"),
      t("设置页有未保存的修改，切换语言会刷新页面并丢失这些修改。"),
      t("切换"),
    ))
  )
    return;
  if (!setLang(lang === "en" ? "zh" : "en")) notify(t("无法保存语言选择，浏览器禁止了本地存储"));
};

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
  preview.textContent = t(frameUrl ? "实时监看" : avatarUrl ? "角色参考图" : "未收到画面");
  preview.classList.toggle("is-live", !!frameUrl);
  el("emptyTitle").textContent = t(configuration?.settings.avatar_id ? "已配置云端形象，等待画面" : "等待角色画面");
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
    const preview = el("avatarPreview");
    if (u) preview.src = u;
    else preview.removeAttribute("src");
    preview.hidden = !u;
    showPoster();
  } catch (e) {
    notify(e.message);
  }
}

function paintEvents(v) {
  const events = v.events;
  el("eventCount").textContent = events.length ? t("{0} 条", events.length) : "";
  el("clearEvents").hidden = !events.length;
  paintFeed(el("events"), events, t("暂无直播事件"));
  const audience = events.filter((event) => /·(?:弹幕|礼物)$/.test(event.kind));
  paintFeed(el("audienceFeed"), audience, t("等待弹幕或礼物"));
}

function paintFeed(list, events, emptyLabel) {
  const stamp = JSON.stringify(events);
  if (list.dataset.stamp === stamp) return;
  list.dataset.stamp = stamp;
  list.replaceChildren();
  if (!events.length) {
    const empty = document.createElement("div");
    empty.className = "empty-events";
    empty.textContent = emptyLabel;
    list.append(empty);
    return;
  }
  for (const e of [...events].reverse()) {
    const item = document.createElement("div"),
      time = document.createElement("time"),
      type = document.createElement("small"),
      p = document.createElement("p");
    time.textContent = new Date(e.at).toLocaleTimeString(lang === "en" ? "en-GB" : "zh-CN", { hour12: false });
    type.textContent = tr(e.kind);
    p.textContent = tr(e.text);
    item.className = "event";
    item.append(time, type, p);
    list.append(item);
  }
}

function paintMetrics(v) {
  const m = v.metrics;
  const latency = el("latency");
  latency.textContent =
    m.last_first_audio_ms === null ? "—" : t("{0} 秒", (m.last_first_audio_ms / 1000).toFixed(2));
  const reply = m.last_reply;
  latency.title = reply ? [
    reply.queue_ms != null && t("排队 {0} ms", reply.queue_ms),
    reply.first_token_ms != null && t("模型首字 {0} ms", reply.first_token_ms),
    reply.first_sentence_ms != null && t("首句 {0} ms", reply.first_sentence_ms),
    reply.tts_ms != null && t("语音合成 {0} ms", reply.tts_ms),
    reply.prompt_tokens != null && t("输入 {0} token", reply.prompt_tokens),
    reply.cached_tokens != null && t("缓存命中 {0} token", reply.cached_tokens),
  ].filter(Boolean).join(" · ") : "";
  el("dimensions").textContent = m.output_width
    ? m.output_width +
      " × " +
      m.output_height +
      (m.output_fps != null ? " · " + m.output_fps.toFixed(1) + " fps" : "")
    : "—";
  el("bill").textContent =
    m.credits != null
      ? t("{0} 积分", m.credits)
      : m.billed_seconds != null
        ? t("{0} 秒", m.billed_seconds)
        : v.phase === "ended"
          ? t("平台未回传金额")
          : "—";
}

function paint(v) {
  state = v;
  paintOnlineBgm(v);
  const live = v.phase === "live";
  const douyin = el("douyinState");
  douyin.textContent = v.douyin ? t("抖音弹幕 · {0}", tr(v.douyin)) : "";
  douyin.classList.toggle("is-ready", /弹幕已接入/.test(v.douyin));
  el("phase").textContent = names[v.phase] ? t(names[v.phase]) : v.phase;
  el("lamp").className = live ? "live" : busy() || v.phase === "unconfirmed" ? "warn" : "";
  el("start").hidden = busy();
  el("start").disabled = v.phase === "unconfirmed";
  el("stop").hidden = !busy();
  el("stop").disabled = v.phase === "stopping";
  el("send").disabled = !live;
  for (const button of quickActions) button.disabled = !live;
  el("interrupt").disabled = !live;
  for (const id of ["reference", "removeRef"]) el(id).disabled = !live;
  paintDance(v);
  el("playTrack").disabled = !live || !el("tracks").value;
  el("toggleBgm").textContent = t(v.bgm_playing || v.current_bgm ? "暂停 BGM" : "开启 BGM");
  el("bgmNow").textContent = !v.current_bgm
    ? t("未播放")
    : !v.output
      ? t("舞台未打开（请点「打开舞台」）")
      : v.singing
        ? t("唱歌已自动暂停 · {0}", v.current_bgm)
        : v.bgm_status === "blocked"
          ? t("请点舞台「允许播放声音」")
          : v.bgm_status === "loading"
            ? t("舞台加载中… {0}", v.current_bgm)
            : v.bgm_status?.startsWith("error:")
              ? t("播放失败 · {0}", v.bgm_status.slice(6))
              : v.bgm_status === "playing"
                ? t("舞台出声中 · {0}", v.current_bgm)
                : t("等待舞台播放 · {0}", v.current_bgm);
  el("soundcheck").disabled = !!busy() || !v.output;
  el("message").textContent = tr(v.message);
  el("outputState").textContent = v.output ? tr(v.output) : t("舞台未打开");
  const stageDot = el("stageDot");
  if (stageDot) stageDot.classList.toggle("is-ready", !!v.output);
  el("clock").textContent = v.started ? timespan((v.ended || Date.now()) - v.started) : "00:00:00";
  el("pending").textContent = v.singing
    ? t("{0} · 演唱《{1}》", v.pending, v.current_song || t("曲目"))
    : v.voice_busy
      ? t("{0} · 播出中", v.pending)
      : String(v.pending);
  const strip = el("signalStrip");
  if (strip) {
    strip.dataset.live = String(live);
    strip.dataset.speaking = String(live && (!!v.voice_busy || !!v.singing));
    const caption = el("signalCaption");
    if (caption) {
      caption.textContent = live
        ? v.singing
          ? t("直播中 · 正在演唱《{0}》", v.current_song || t("曲目"))
          : t(v.voice_busy ? "直播中 · 语音播出" : "数字人已连线")
        : t(busy() ? "连线中 · 建立信令" : v.output ? "待机 · 舞台已连接" : "待机 · 本机就绪");
    }
  }
  paintMetrics(v);
  el("recovery").hidden = v.phase !== "unconfirmed";
  el("recordRow").hidden = !(v.phase === "ended" && configuration?.settings.recording);
  if (!live && !v.dance?.started && frameUrl) {
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
    if ((v.phase === "live" || v.dance?.started) && !visibleConfig && timer % 2 === 0) {
      const r = await request("/api/monitor");
      if (frameUrl) URL.revokeObjectURL(frameUrl);
      frameUrl = r.status === 204 ? "" : URL.createObjectURL(await r.blob());
      showPoster();
    }
    if (visibleConfig && (servicesStarting || timer % 6 === 0)) await voiceStatus();
    if (visibleConfig && timer % 4 === 0) void obsStream();
  } catch (e) {
    el("phase").textContent = t("本机连接中断");
    el("lamp").className = "warn";
    el("message").textContent = e.message;
    el("send").disabled = true;
    el("start").disabled = true;
  } finally {
    collecting = false;
  }
}

function syncLiveAudioControls() {
  const s = configuration?.settings;
  if (!s) return;
  if (el("liveSongVol")) {
    el("liveSongVol").value = s.song_volume ?? 95;
    el("liveSongVolVal").textContent = (s.song_volume ?? 95) + "%";
  }
  if (el("liveSyncOffset")) {
    el("liveSyncOffset").value = s.song_sync_offset_ms ?? 800;
  }
  if (el("liveBgmVol")) {
    el("liveBgmVol").value = s.bgm_volume ?? 25;
    el("liveBgmVolVal").textContent = (s.bgm_volume ?? 25) + "%";
  }
}

setupConfig(() => {
  refreshAvatar();
  syncLiveAudioControls();
  if (state) paint(state);
});

action("chooseRole", () => tab("config"));

action("start", async () => {
  if (!configuration.keys.vidu || (!configuration.avatar && !configuration.settings.avatar_id)) {
    tab("config");
    notify(t("开播需填写 Vidu 密钥，并上传角色图或填写形象 ID"));
    return;
  }
  const min = configuration.settings.max_minutes;
  if (
    !(await confirmAction(
      t("开始云端连线"),
      t("当前时限 {0} 分钟。按组件版公开价折合约 ¥{1}，实际结算以 Vidu 为准。", min, (min * 1.875).toFixed(2)),
      t("开始连线"),
    ))
  )
    return;
  await api("/api/start", { keep_events: !!el("keepEventsOnStart")?.checked });
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
      t("确认云端已结束"),
      t("仅在 Vidu 控制台确认此场次已经结束后使用。Vorela 会保留人工核对记录。"),
      t("确认已结束"),
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
  await copyText(stageUrl());
  const btn = el("copyStage");
  btn.classList.add("is-copied");
  setButtonLabel(btn, t("已复制地址"));
  clearTimeout(copyTimer);
  copyTimer = setTimeout(() => {
    btn.classList.remove("is-copied");
    setButtonLabel(btn, t("复制 OBS 地址"));
  }, 1800);
  notify(t("已复制 OBS 浏览器源地址"));
});

action("openStage", async () => {
  const w = window.open("about:blank", "vorela_stage");
  if (!w) throw Error(t("请允许浏览器打开舞台窗口"));
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
  el("line").placeholder = t({ say: "输入要说的话", reply: "输入给角色的问题", action: "输入角色要执行的动作" }[mode]);
  el("send").textContent = t(mode === "action" ? "发送动作" : "加入播出");
  el("quickActions").hidden = mode !== "action";
  el("soundcheck").hidden = mode !== "say";
  el("audition").hidden = mode === "action";
  setButtonLabel(el("audition"), t(mode === "reply" ? "试听回复" : "本机试听"));
};

const quickActions = el("quickActions").querySelectorAll("button");
for (const button of quickActions) {
  action(button.id, async () => {
    await api("/api/command", { kind: "action", text: button.dataset.action });
    notify(t("已发送动作 · {0}", button.textContent));
    await tick();
  });
}

el("line").addEventListener("input", updateCharCount);

action("send", async () => {
  const text = el("line").value.trim();
  if (!text) throw Error(t("请输入播出内容"));
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
  if (!text) throw Error(t("请输入试音文字"));
  await api("/api/soundcheck", { text });
  notify(t("已向独立舞台发送试音"));
  await tick();
});

let audioUrl = "";
action("audition", async () => {
  const text = el("line").value.trim();
  if (!text) throw Error(t("请输入要试听的文字"));
  setButtonLabel(el("audition"), t("合成中…"));
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
    setButtonLabel(el("audition"), t(mode === "reply" ? "试听回复" : "本机试听"));
  }
});

async function fileEvent(input, path) {
  const file = input.files[0];
  if (!file) return;
  try {
    await request(path, { method: "POST", body: file });
    notify(t("已提交"));
    await tick();
  } catch (e) {
    notify(e.message);
  } finally {
    input.value = "";
  }
}
el("reference").onchange = (e) => fileEvent(e.target, "/api/reference/" + el("refKind").value);

const AUDIO_EXT_RE = /\.(mp3|wav|flac|m4a|aac|ogg|opus|aiff|aif|wma|webm|mp4)$/i;
function isImportableAudio(file, allowMp4) {
  if (!file || !file.name || file.name.startsWith(".")) return false;
  if (file.type && file.type.startsWith("audio/")) return true;
  if (allowMp4 && file.type === "video/mp4") return true;
  if (!allowMp4 && /\.mp4$/i.test(file.name)) return false;
  return AUDIO_EXT_RE.test(file.name);
}

function fileExt(name) {
  const m = /\.([^.]+)$/.exec(name);
  return m ? m[1].toLowerCase() : "";
}

async function importAudioBatch(rawFiles, input, { endpoint, statusId, detailsId, allowMp4, onDone }) {
  const files = Array.from(rawFiles)
    .filter((f) => isImportableAudio(f, allowMp4))
    .sort((a, b) => a.name.localeCompare(b.name, "zh-CN", { numeric: true }));
  if (!files.length) {
    notify(t("所选内容中未找到可导入的音频文件"));
    input.value = "";
    return;
  }
  const status = el(statusId);
  const details = el(detailsId);
  const errors = details.querySelector("ul");
  let imported = 0;
  let skipped = 0;
  let failed = 0;
  input.disabled = true;
  status.hidden = false;
  details.hidden = true;
  details.open = false;
  errors.replaceChildren();
  try {
    for (const [index, file] of files.entries()) {
      status.textContent = t("正在导入第 {0} / {1} 首…", index + 1, files.length);
      const title = file.name.replace(/\.[^.]+$/, "");
      const ext = fileExt(file.name);
      const headers = ext ? { "X-File-Ext": ext } : undefined;
      try {
        await request(endpoint + encodeURIComponent(title), {
          method: "PUT",
          headers,
          body: file,
        });
        imported += 1;
      } catch (error) {
        if (/已存在/.test(error.raw ?? error.message)) {
          skipped += 1;
        } else {
          failed += 1;
          const item = document.createElement("li");
          item.textContent = `${file.name}：${error.message}`;
          errors.append(item);
        }
      }
    }
    const parts = [t("新增 {0} 首", imported)];
    if (skipped) parts.push(t("跳过已存在 {0} 首", skipped));
    if (failed) parts.push(t("失败 {0} 首", failed));
    status.textContent = parts.join(" · ");
    details.hidden = failed === 0;
    if (imported) await onDone();
  } catch (error) {
    notify(error.message);
  } finally {
    input.value = "";
    input.disabled = false;
  }
}

async function loadTracks() {
  const items = await api("/api/tracks"),
    select = el("tracks"),
    old = select.value;
  select.replaceChildren();
  if (!items.length) select.add(new Option(t("曲库为空"), ""));
  for (const track of items) {
    const tag = track.source_ext ? t(" · 原音质") : "";
    select.add(new Option(track.title + " · " + timespan(track.duration_ms) + tag, track.id));
  }
  if (items.some((track) => track.id === old)) select.value = old;
  el("trackCount").textContent = items.length ? t("共 {0} 首", items.length) : "";
  el("removeTrack").disabled = !items.length;
  el("clearTracks").disabled = !items.length;
  el("previewTrack").disabled = !items.length;
  el("playTrack").disabled = !items.length || state?.phase !== "live";
}

el("addTrack").onchange = (e) =>
  importAudioBatch(e.currentTarget.files, e.currentTarget, {
    endpoint: "/api/tracks/",
    statusId: "trackImportStatus",
    detailsId: "trackImportErrors",
    allowMp4: true,
    onDone: loadTracks,
  });

el("addTrackFolder").onchange = (e) =>
  importAudioBatch(e.currentTarget.files, e.currentTarget, {
    endpoint: "/api/tracks/",
    statusId: "trackImportStatus",
    detailsId: "trackImportErrors",
    allowMp4: true,
    onDone: loadTracks,
  });

el("addBgm").onchange = (e) =>
  importAudioBatch(e.currentTarget.files, e.currentTarget, {
    endpoint: "/api/bgm/",
    statusId: "bgmImportStatus",
    detailsId: "bgmImportErrors",
    allowMp4: false,
    onDone: loadBgm,
  });

el("addBgmFolder").onchange = (e) =>
  importAudioBatch(e.currentTarget.files, e.currentTarget, {
    endpoint: "/api/bgm/",
    statusId: "bgmImportStatus",
    detailsId: "bgmImportErrors",
    allowMp4: false,
    onDone: loadBgm,
  });

action("playTrack", async () => {
  await api("/api/tracks/" + el("tracks").value, {});
  notify(t("曲目已加入演唱队列"));
});

async function previewAudioWithFallback(primaryUrl, wavUrl) {
  const player = el("audio");
  const playUrl = async (u) => {
    const r = await request(u);
    if (audioUrl) URL.revokeObjectURL(audioUrl);
    audioUrl = URL.createObjectURL(await r.blob());
    player.src = audioUrl;
    player.hidden = false;
    await player.play();
  };
  try {
    await playUrl(primaryUrl);
  } catch (err) {
    if (err?.name !== "NotAllowedError" && wavUrl) {
      await playUrl(wavUrl);
    } else {
      throw err;
    }
  }
}

action("previewTrack", async () => {
  const id = el("tracks").value;
  if (!id) return;
  await previewAudioWithFallback(
    "/api/tracks/" + encodeURIComponent(id) + "/audio",
    "/api/tracks/" + encodeURIComponent(id) + "/wav",
  );
});

action("removeTrack", async () => {
  if (await confirmAction(t("从演唱曲库移除"), t("移除所选曲目在 Vorela 中的副本。"), t("移除"))) {
    await request("/api/tracks/" + el("tracks").value, { method: "DELETE" });
    await loadTracks();
  }
});

action("clearTracks", async () => {
  if (await confirmAction(t("清空演唱曲库"), t("移除演唱曲库中的全部歌曲副本（不影响原始文件）。"), t("清空全部"))) {
    await request("/api/tracks", { method: "DELETE" });
    await loadTracks();
    notify(t("已清空演唱曲库"));
  }
});

function loadBgm() { return loadBgmList(timespan); }
setupOnlineBgm(loadBgm, tick);

action("toggleBgm", async () => {
  const res = await api("/api/bgm/control", { action: "toggle" });
  await tick();
  if (!res.bgm?.enabled) {
    notify(t("背景音乐已暂停"));
  } else if (!state?.output) {
    notify(t("背景音乐已开启，请先点「打开舞台」或在 OBS 加载舞台以收听声音"));
  } else if (state?.output?.includes("允许播放")) {
    notify(t("背景音乐已开启，请切到舞台页点击「恢复声音」允许播放"));
  } else {
    notify(t("背景音乐已开启（由舞台 / OBS 输出）"));
  }
});

action("previewBgm", async () => {
  const id = el("bgmTracks").value;
  if (!id) return;
  await previewAudioWithFallback(
    "/api/bgm/" + encodeURIComponent(id) + "/audio",
    "/api/bgm/" + encodeURIComponent(id) + "/wav",
  );
});

el("bgmTracks").onchange = async () => {
  const id = el("bgmTracks").value;
  if (id && (state?.bgm_playing || state?.current_bgm)) {
    try {
      await api("/api/bgm/control", { action: "select", id });
      await tick();
    } catch (e) {
      notify(e.message);
    }
  }
};

let liveAudioTimer = 0;
function pushLiveAudio(patch) {
  if (configuration?.settings) Object.assign(configuration.settings, patch);
  clearTimeout(liveAudioTimer);
  liveAudioTimer = setTimeout(async () => {
    try {
      await api("/api/audio-live", patch);
    } catch (e) {
      notify(e.message);
    }
  }, 180);
}

el("liveSongVol").oninput = (e) => {
  const v = Number(e.target.value);
  el("liveSongVolVal").textContent = v + "%";
  pushLiveAudio({ song_volume: v });
};

el("liveSyncOffset").onchange = (e) => {
  const v = Math.max(0, Math.min(4000, Math.round(Number(e.target.value) || 0)));
  e.target.value = v;
  pushLiveAudio({ song_sync_offset_ms: v });
};

el("liveBgmVol").oninput = (e) => {
  const v = Number(e.target.value);
  el("liveBgmVolVal").textContent = v + "%";
  pushLiveAudio({ bgm_volume: v });
};

action("removeRef", () => api("/api/command", { kind: "remove_reference" }));

async function sendSimulated(kind) {
  const platform = el("simPlatform").value;
  const author = el("simAuthor").value.trim();
  const text = el("simText").value.trim();
  const payload = { platform, kind, author };
  if (kind === "gift") {
    payload.gift_name = el("simGift").value.trim();
    payload.gift_count = Math.max(1, Number(el("simCount").value) || 1);
    if (text) payload.text = text;
  } else if (kind === "chat") {
    payload.text = text;
  }
  await api("/api/simulate", payload);
  await tick();
  notify(
    state?.phase === "live"
      ? t("已触发模拟事件")
      : t("已写入动态日志（未开播不会排入播报；可点「清空动态」清理）"),
  );
}
action("simJoin", () => sendSimulated("join"));
action("simGiftBtn", () => sendSimulated("gift"));
action("simChat", () => sendSimulated("chat"));
action("clearEvents", async () => {
  await api("/api/events/clear", {});
  await tick();
  notify(t("已清空直播动态"));
});
if (el("keepEventsOnStart")) {
  el("keepEventsOnStart").checked = localStorage.getItem("vorela.keepEvents") === "1";
  el("keepEventsOnStart").onchange = (e) => {
    localStorage.setItem("vorela.keepEvents", e.target.checked ? "1" : "0");
  };
}

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
    notify(t("云端录播尚未返回下载地址"));
    return;
  }
  urls.forEach((u, i) => {
    const a = document.createElement("a");
    a.href = u;
    a.target = "_blank";
    a.rel = "noreferrer";
    a.textContent = t("下载录播 {0}", i + 1);
    el("recordLinks").append(a);
  });
});

updateCharCount();

if (!token) {
  notify(t("请从 Vorela 应用打开控制台"));
  el("phase").textContent = t("需要本机授权");
  el("lamp").className = "warn";
  el("start").disabled = true;
} else {
  try {
    await loadConfig();
    await loadDances();
    await loadTracks();
    await loadBgm();
    await tick();
    setInterval(tick, 1200);
  } catch (e) {
    notify(e.message);
  }
}
