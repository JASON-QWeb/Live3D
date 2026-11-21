sessionStorage.removeItem("avatar3d.admin");
const supplied = location.hash.slice(1);
if (supplied) {
  sessionStorage.setItem("avatar3d.stage", supplied);
  history.replaceState(null, "", location.pathname);
}
let token = sessionStorage.getItem("avatar3d.stage") || "";
const client = crypto.randomUUID();
const picture = document.getElementById("picture"),
  caption = document.getElementById("sub"),
  resume = document.getElementById("resume"),
  stageNotice = document.getElementById("stageNotice");
let rtc = null,
  desired = "",
  joining = false,
  generation = 0,
  status = "standby",
  resumes = [],
  sdkTask,
  retryAt = 0;
let lastFrameAt = 0,
  lastFrames = 0,
  lastTime = 0,
  frameRate = null,
  width = null,
  height = null,
  stageOwner = false,
  pollBusy = false;
function loadSdk() {
  if (window.TRTC) return Promise.resolve(window.TRTC);
  if (!sdkTask)
    sdkTask = new Promise((resolve, reject) => {
      const script = document.createElement("script");
      script.src = "/sdk/trtc.js";
      script.integrity = "sha384-ly1Gkjo/1LyiaAdx8mfZ4N/BpBIgFNVQK4FvKp25yli3N1IZPzfrEXSU/7ijlVfr";
      script.onload = () => (window.TRTC ? resolve(window.TRTC) : reject(Error("SDK unavailable")));
      script.onerror = () => {
        script.remove();
        reject(Error("SDK load failed"));
      };
      document.head.append(script);
    }).catch((e) => {
      sdkTask = null;
      throw e;
    });
  return sdkTask;
}
function clearScreen() {
  caption.textContent = "";
  picture.replaceChildren();
  lastFrameAt = 0;
  lastFrames = 0;
  lastTime = 0;
  frameRate = width = height = null;
  resumes = [];
  resume.hidden = true;
}
async function leave() {
  generation++;
  const old = rtc;
  rtc = null;
  desired = "";
  joining = false;
  clearScreen();
  status = "standby";
  if (old) {
    try {
      await old.exitRoom();
    } catch {}
    try {
      old.destroy();
    } catch {}
  }
}
function room(id) {
  const n = Number(id);
  if (String(n) === String(id) && Number.isInteger(n) && n > 0 && n < 4294967295) return { roomId: n };
  return { strRoomId: String(id) };
}
async function join(id, cred) {
  if (joining || Date.now() < retryAt) return;
  joining = true;
  desired = id;
  const version = ++generation;
  status = "joining";
  let instance;
  try {
    const T = await loadSdk();
    if (version !== generation) return;
    instance = T.create();
    rtc = instance;
    instance.on(T.EVENT.REMOTE_VIDEO_AVAILABLE, async ({ userId, streamType }) => {
      if (version !== generation) return;
      try {
        await instance.startRemoteVideo({ userId, streamType, view: picture });
      } catch {
        if (version === generation) status = "rtc_error";
      }
    });
    instance.on(T.EVENT.REMOTE_VIDEO_UNAVAILABLE, () => {
      if (version === generation) {
        status = "lost";
        lastFrameAt = 0;
      }
    });
    instance.on(T.EVENT.AUTOPLAY_FAILED, (e) => {
      if (version === generation) {
        resumes.push(e.resume);
        resume.hidden = false;
        status = "audio_blocked";
      }
    });
    instance.on(T.EVENT.ERROR, () => {
      if (version === generation) status = "rtc_error";
    });
    instance.on(T.EVENT.KICKED_OUT, () => {
      if (version === generation) {
        leave();
        retryAt = Date.now() + 10000;
      }
    });
    await instance.enterRoom({
      sdkAppId: Number(cred.app_id),
      userId: String(cred.user_id),
      userSig: cred.token,
      ...room(cred.channel_id),
      scene: T.TYPE.SCENE_LIVE,
      role: T.TYPE.ROLE_AUDIENCE,
      autoReceiveAudio: true,
      autoReceiveVideo: true,
      enableAutoPlayDialog: false,
    });
    if (version === generation && status === "joining") status = "waiting";
  } catch {
    if (version === generation) {
      const reason = window.TRTC ? "rtc_error" : "sdk_error";
      await leave();
      status = reason;
      retryAt = Date.now() + 5000;
    }
  } finally {
    if (version === generation) joining = false;
  }
}
resume.onclick = async () => {
  try {
    await Promise.all(resumes.map((fn) => fn()));
    resumes = [];
    resume.hidden = true;
    status = "waiting";
  } catch {
    status = "audio_blocked";
  }
};
async function frame(video) {
  const c = document.createElement("canvas");
  c.width = Math.min(480, video.videoWidth);
  c.height = Math.max(1, Math.round((video.videoHeight * c.width) / video.videoWidth));
  c.getContext("2d").drawImage(video, 0, 0, c.width, c.height);
  const blob = await new Promise((resolve) => c.toBlob(resolve, "image/jpeg", 0.65));
  if (blob && desired && stageOwner) {
    await fetch("/api/frame/" + client + "/" + encodeURIComponent(desired), {
      method: "PUT",
      headers: { Authorization: "Bearer " + token },
      body: blob,
    });
  }
}
function observe() {
  const video = picture.querySelector("video");
  if (!video || video.readyState < 2 || !video.videoWidth || video.paused || !rtc) return;
  const t = performance.now(),
    count = video.getVideoPlaybackQuality?.().totalVideoFrames;
  const advanced = count == null ? video.currentTime !== lastTime : count > lastFrames;
  if (advanced) {
    frameRate = count != null && lastFrameAt > 0 ? ((count - lastFrames) * 1000) / (t - lastFrameAt) : null;
    width = video.videoWidth;
    height = video.videoHeight;
    lastFrameAt = t;
    lastTime = video.currentTime;
    lastFrames = count || 0;
    if (!resumes.length) status = "playing";
    frame(video).catch(() => {});
  } else if (lastFrameAt && t - lastFrameAt > 5000) {
    status = "lost";
    frameRate = 0;
  }
}
let checkId = "",
  checkUrl = "",
  checkPlayer = null;
function stopCheck() {
  if (checkPlayer) {
    checkPlayer.pause();
    checkPlayer.onended = null;
    checkPlayer = null;
  }
  if (checkUrl) {
    URL.revokeObjectURL(checkUrl);
    checkUrl = "";
  }
  checkId = "";
}
async function audition(id) {
  if (id === checkId) return;
  stopCheck();
  if (!id) return;
  checkId = id;
  try {
    const r = await fetch("/api/test-audio/" + encodeURIComponent(id), {
      headers: { Authorization: "Bearer " + token },
    });
    if (!r.ok) throw Error("试音未就绪");
    const blob = await r.blob();
    if (checkId !== id) return;
    checkUrl = URL.createObjectURL(blob);
    const player = new Audio(checkUrl);
    checkPlayer = player;
    player.onended = () => {
      fetch("/api/test-audio/" + encodeURIComponent(id) + "/" + client, {
        method: "POST",
        headers: { Authorization: "Bearer " + token },
      }).catch(() => {});
    };
    try {
      await player.play();
    } catch {
      resumes.push(() => player.play());
      resume.hidden = false;
      status = "audio_blocked";
    }
  } catch {
    if (checkId === id) checkId = "";
  }
}
let rtcSupported = null,
  lastProbe = 0;
async function probeReceiver() {
  if (rtcSupported !== null || Date.now() - lastProbe < 60000) return;
  lastProbe = Date.now();
  try {
    const T = await loadSdk();
    const report = await T.isSupported();
    rtcSupported = report.detail.isWebRTCSupported && report.detail.isH264DecodeSupported;
  } catch {
    // The stage will retry a failed download while it is idle.
  }
}
const pack = () => ({
  client,
  status,
  live_id: desired || null,
  fps: frameRate,
  width,
  height,
  rtc_supported: rtcSupported,
});
async function poll() {
  if (!token || pollBusy) return;
  pollBusy = true;
  try {
    observe();
    const r = await fetch("/api/output", {
      method: "POST",
      headers: { Authorization: "Bearer " + token, "Content-Type": "application/json" },
      body: JSON.stringify(pack()),
    });
    if (!r.ok) throw Error("output rejected");
    const v = await r.json();
    stageOwner = v.owner;
    if (!v.owner) {
      stopCheck();
      if (rtc || desired) await leave();
      document.title = "Avatar3D · 舞台已被占用";
      return;
    }
    document.title = "Avatar3D · 舞台输出";
    if (!v.rtc || !v.live_id) {
      if (rtc || desired) await leave();
      probeReceiver();
      caption.textContent = "";
      audition(v.check || "");
      return;
    }
    if (checkId) stopCheck();
    if (desired && desired !== v.live_id) await leave();
    caption.textContent = v.subtitle || "";
    if (!rtc && !joining) join(v.live_id, v.rtc);
  } catch {
    if (rtc || desired) await leave();
  } finally {
    pollBusy = false;
  }
}
window.addEventListener("pagehide", () => {
  if (!stageOwner) return;
  fetch("/api/output", {
    method: "POST",
    keepalive: true,
    headers: { Authorization: "Bearer " + token, "Content-Type": "application/json" },
    body: JSON.stringify({ ...pack(), status: "released" }),
  }).catch(() => {});
});
let pollTimer = 0;
function startPolling() {
  if (stageNotice) stageNotice.hidden = true;
  document.body.classList.remove("no-token");
  poll();
  if (!pollTimer) pollTimer = setInterval(poll, 1000);
}
if (token) {
  startPolling();
} else if (stageNotice) {
  document.body.classList.add("no-token");
  stageNotice.hidden = false;
}
window.addEventListener("hashchange", () => {
  const next = location.hash.slice(1);
  if (!next) return;
  token = next;
  sessionStorage.setItem("avatar3d.stage", next);
  history.replaceState(null, "", location.pathname);
  startPolling();
});
