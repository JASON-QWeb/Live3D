import { dancePlayer } from "./stage-dance.js";
sessionStorage.removeItem("vorela.admin");
const supplied = location.hash.slice(1);
if (supplied) {
  sessionStorage.setItem("vorela.stage", supplied);
  history.replaceState(null, "", location.pathname);
}
let token = sessionStorage.getItem("vorela.stage") || "";
const client = crypto.randomUUID();
const picture = document.getElementById("picture"),
  caption = document.getElementById("sub"),
  resume = document.getElementById("resume"),
  stageNotice = document.getElementById("stageNotice"),
  occupiedNotice = document.getElementById("occupiedNotice"),
  takeoverBtn = document.getElementById("takeoverStage"),
  songAudioEl = document.getElementById("songAudio"),
  bgmAudioEl = document.getElementById("bgmAudio");
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
    instance.on(T.EVENT.REMOTE_AUDIO_AVAILABLE, () => {
      if (version === generation && (trtcAudioMuted || danceAudio)) {
        applyTrtcAudioMute(trtcAudioMuted);
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
    if (version === generation && (trtcAudioMuted || danceAudio)) {
      applyTrtcAudioMute(trtcAudioMuted);
    }
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
    status = rtc ? "waiting" : "standby";
  } catch {
    status = "audio_blocked";
  }
};
async function frame(video, id = desired) {
  const c = document.createElement("canvas");
  c.width = Math.min(480, video.videoWidth);
  c.height = Math.max(1, Math.round((video.videoHeight * c.width) / video.videoWidth));
  c.getContext("2d").drawImage(video, 0, 0, c.width, c.height);
  const blob = await new Promise((resolve) => c.toBlob(resolve, "image/jpeg", 0.65));
  if (blob && id && stageOwner) {
    await fetch("/api/frame/" + client + "/" + encodeURIComponent(id), {
      method: "PUT",
      headers: { Authorization: "Bearer " + token },
      body: blob,
    });
  }
}
function observe() {
  const overlay = dance.visible();
  if (overlay) frame(overlay.video, overlay.id).catch(() => {});
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
let bgmStatusText = "idle";
let forceTakeover = false;
const pack = () => ({
  client,
  status,
  live_id: desired || null,
  fps: frameRate,
  width,
  height,
  rtc_supported: rtcSupported,
  force: forceTakeover,
  bgm_status: bgmStatusText,
});
let trtcAudioMuted = false, danceAudio = false;
const dance = dancePlayer(client, () => token, active => {
  danceAudio = active;
  bgmAudioEl.muted = songAudioEl.muted = active;
  applyTrtcAudioMute(trtcAudioMuted);
});
function applyTrtcAudioMute(muted) {
  trtcAudioMuted = muted;
  muted ||= danceAudio;
  if (rtc) {
    try {
      rtc.muteRemoteAudio("*", muted);
    } catch {}
  }
  for (const el of picture.querySelectorAll("audio, video")) {
    try {
      el.muted = muted;
    } catch {}
  }
}

const clampVol = (v) => Math.max(0, Math.min(1, Number.isFinite(v) ? v : 1));

async function fetchAudioBlob(url) {
  const r = await fetch(url, { headers: { Authorization: "Bearer " + token } });
  if (!r.ok) throw Error("HTTP " + r.status);
  return r.blob();
}

// Singing audio
let songPlaybackId = "";
let songTrackId = "";
let songUrl = "";
let songPlaying = false;
let songPreparePromise = null;
let songUsedWav = false;

function disposeSongPlayer() {
  if (songAudioEl) {
    songAudioEl.pause();
    songAudioEl.onended = null;
    songAudioEl.removeAttribute("src");
    songAudioEl.load();
  }
  if (songUrl) {
    URL.revokeObjectURL(songUrl);
    songUrl = "";
  }
}

function stopSong() {
  songPlaybackId = "";
  songTrackId = "";
  songPreparePromise = null;
  songPlaying = false;
  songUsedWav = false;
  disposeSongPlayer();
  applyTrtcAudioMute(false);
  applyBgmState();
}

function prepareSong(msg, useWav = false) {
  if (!stageOwner || !msg?.playback_id || !msg?.track_id) return Promise.resolve();
  pauseBgmNow();
  if (!useWav && songPlaybackId === msg.playback_id && songPreparePromise) {
    if (songAudioEl && msg.volume != null) songAudioEl.volume = clampVol(msg.volume);
    return songPreparePromise;
  }
  disposeSongPlayer();
  songPlaying = false;
  const pid = msg.playback_id;
  songPlaybackId = pid;
  songTrackId = msg.track_id;
  songUsedWav = useWav;
  const task = (async () => {
    try {
      const path =
        "/api/stage/track/" + encodeURIComponent(msg.track_id) + (useWav ? "/wav" : "");
      const blob = await fetchAudioBlob(path);
      if (songPlaybackId !== pid) return;
      songUrl = URL.createObjectURL(blob);
      songAudioEl.src = songUrl;
      songAudioEl.volume = clampVol(msg.volume ?? 0.95);
      songAudioEl.onended = () => {
        if (songPlaybackId === pid) stopSong();
      };
      songAudioEl.load();
    } catch {
      if (songPlaybackId === pid) stopSong();
    }
  })();
  songPreparePromise = task;
  return task;
}

async function playSong(msg) {
  if (!stageOwner || !msg?.playback_id || !msg?.track_id) return;
  const pid = msg.playback_id;
  if (songPlaybackId !== pid || !songUrl) {
    await prepareSong(msg, false);
  }
  if (songPlaybackId !== pid || !songUrl) return;
  songPlaying = true;
  applyBgmState();
  applyTrtcAudioMute(true);
  songAudioEl.volume = clampVol(msg.volume ?? songAudioEl.volume);
  if (msg.elapsed_ms > 150) {
    try {
      songAudioEl.currentTime = msg.elapsed_ms / 1000;
    } catch {}
  }
  try {
    await songAudioEl.play();
  } catch (err) {
    if (err?.name !== "NotAllowedError" && !songUsedWav) {
      await prepareSong(msg, true);
      if (songPlaybackId === pid && songUrl) {
        try {
          await songAudioEl.play();
          return;
        } catch (err2) {
          if (err2?.name !== "NotAllowedError") {
            stopSong();
            return;
          }
        }
      }
    }
    resumes.push(() => (songUrl ? songAudioEl.play() : Promise.resolve()));
    resume.hidden = false;
    status = "audio_blocked";
  }
}

// =========================================================================
// Background Music (BGM) Player — Never drives Vidu; auto-ducks & auto-pauses
// =========================================================================
let bgmState = null;
let bgmUrl = "";
let bgmLoadedTrackId = "";
let bgmLoadingTrackId = "";
let bgmLoadSeq = 0;
let bgmRampTimer = 0;
let bgmAutoplayBlocked = false;

function pauseBgmNow() {
  clearInterval(bgmRampTimer);
  if (bgmAudioEl && !bgmAudioEl.paused) {
    bgmAudioEl.pause();
  }
  if (bgmState?.enabled) bgmStatusText = "paused";
}

function rampBgmVolume(target, ms = 240) {
  if (!bgmAudioEl) return;
  clearInterval(bgmRampTimer);
  const start = bgmAudioEl.volume;
  const dest = clampVol(target);
  if (Math.abs(dest - start) < 0.01) {
    bgmAudioEl.volume = dest;
    return;
  }
  const steps = Math.max(1, Math.round(ms / 25));
  let step = 0;
  bgmRampTimer = setInterval(() => {
    if (!bgmAudioEl) {
      clearInterval(bgmRampTimer);
      return;
    }
    step++;
    if (step >= steps) {
      bgmAudioEl.volume = dest;
      clearInterval(bgmRampTimer);
    } else {
      bgmAudioEl.volume = clampVol(start + ((dest - start) * step) / steps);
    }
  }, 25);
}

function stopBgm() {
  bgmLoadSeq++;
  bgmLoadingTrackId = "";
  bgmAutoplayBlocked = false;
  bgmStatusText = "idle";
  clearInterval(bgmRampTimer);
  if (bgmAudioEl) {
    bgmAudioEl.pause();
    bgmAudioEl.onended = null;
    bgmAudioEl.removeAttribute("src");
    bgmAudioEl.load();
  }
  if (bgmUrl) {
    URL.revokeObjectURL(bgmUrl);
    bgmUrl = "";
  }
  bgmLoadedTrackId = "";
}

function queueBgmResume() {
  bgmStatusText = "blocked";
  if (bgmAutoplayBlocked) return;
  bgmAutoplayBlocked = true;
  resumes.push(async () => {
    bgmAutoplayBlocked = false;
    if (!bgmUrl || !bgmState?.enabled || bgmState?.paused_for_song || songPlaying || songPlaybackId) {
      return;
    }
    await bgmAudioEl.play();
    bgmStatusText = "playing";
  });
  resume.hidden = false;
  status = "audio_blocked";
}

async function startBgmPlay(trackId, seq, targetVol, usedWav) {
  if (seq !== bgmLoadSeq || !stageOwner) return;
  if (bgmState?.paused_for_song || songPlaying || songPlaybackId || !bgmState?.enabled) {
    bgmStatusText = "paused";
    return;
  }
  try {
    await bgmAudioEl.play();
    if (seq === bgmLoadSeq) {
      bgmStatusText = "playing";
      bgmAutoplayBlocked = false;
      if (status === "audio_blocked" && !resumes.length) {
        status = rtc ? "waiting" : "standby";
      }
    }
  } catch (err) {
    if (seq !== bgmLoadSeq) return;
    if (err?.name === "NotAllowedError") {
      queueBgmResume();
      return;
    }
    // Unsupported browser codec (e.g. FLAC/Ogg in Safari): auto-fallback to universal 48kHz stereo WAV!
    if (!usedWav) {
      bgmStatusText = "loading";
      try {
        const wavBlob = await fetchAudioBlob(
          "/api/stage/bgm/" + encodeURIComponent(trackId) + "/wav",
        );
        if (seq !== bgmLoadSeq || !stageOwner) return;
        if (bgmUrl) URL.revokeObjectURL(bgmUrl);
        bgmUrl = URL.createObjectURL(wavBlob);
        bgmAudioEl.src = bgmUrl;
        bgmAudioEl.volume = targetVol;
        bgmAudioEl.load();
        await startBgmPlay(trackId, seq, targetVol, true);
        return;
      } catch (wavErr) {
        if (seq !== bgmLoadSeq) return;
        if (bgmState?.online) await advanceStageBgm(bgmState, true).catch(() => {});
        bgmStatusText = "error:" + (wavErr?.message || "音频转码失败");
        return;
      }
    }
    bgmStatusText = "error:" + (err?.name || "无法播放");
    if (bgmState?.online) await advanceStageBgm(bgmState, true).catch(() => {});
  }
}

let advancingRevision = null;
async function advanceStageBgm(st, failed = false) {
  if (advancingRevision === st.revision || !stageOwner) return;
  advancingRevision = st.revision;
  try {
    const response = await fetch("/api/stage/bgm/next", {
      method: "POST",
      headers: { Authorization: "Bearer " + token, "Content-Type": "application/json" },
      body: JSON.stringify({ client, revision: st.revision, failed: failed ? st.track_id : null }),
    });
    if (!response.ok) throw new Error(response.statusText);
    const result = await response.json();
    if (result.bgm && bgmState?.revision === st.revision) applyBgmState(result.bgm);
  } finally {
    if (advancingRevision === st.revision) advancingRevision = null;
  }
}

async function applyBgmState(nextState) {
  if (nextState) bgmState = nextState;
  const st = bgmState;
  if (!stageOwner || !st || !st.enabled || !st.track_id) {
    stopBgm();
    return;
  }
  if (advancingRevision === st.revision) return;
  const pausedForSong = !!st.paused_for_song || songPlaying || !!songPlaybackId;
  if (pausedForSong) {
    pauseBgmNow();
    return;
  }
  const baseVol = clampVol(st.volume ?? 0.25);
  const duckRatio = clampVol(st.duck_percent ?? 0.3);
  const targetVol = st.ducked ? baseVol * duckRatio : baseVol;

  if (bgmLoadedTrackId === st.track_id && bgmUrl) {
    if (bgmAudioEl.paused && !bgmAutoplayBlocked) {
      bgmAudioEl.volume = Math.min(bgmAudioEl.volume, targetVol);
      await startBgmPlay(st.track_id, bgmLoadSeq, targetVol, false);
    }
    if (!bgmAudioEl.paused) {
      bgmStatusText = "playing";
      rampBgmVolume(targetVol, st.ducked ? 180 : 320);
    }
    return;
  }

  // Already fetching this exact track — do NOT abort & restart on 1s heartbeat polls!
  if (bgmLoadingTrackId === st.track_id) {
    return;
  }

  const seq = ++bgmLoadSeq;
  const trackId = st.track_id;
  bgmLoadingTrackId = trackId;
  bgmAutoplayBlocked = false;
  bgmStatusText = "loading";
  clearInterval(bgmRampTimer);
  bgmAudioEl.pause();
  bgmAudioEl.onended = null;
  if (bgmUrl) {
    URL.revokeObjectURL(bgmUrl);
    bgmUrl = "";
  }
  bgmLoadedTrackId = "";
  try {
    const blob = await fetchAudioBlob("/api/stage/bgm/" + encodeURIComponent(trackId));
    if (seq !== bgmLoadSeq || !stageOwner) return;
    bgmUrl = URL.createObjectURL(blob);
    bgmAudioEl.src = bgmUrl;
    bgmAudioEl.volume = targetVol;
    bgmAudioEl.onended = () => {
      if (seq !== bgmLoadSeq) return;
      bgmLoadedTrackId = "";
      bgmStatusText = "loading";
      advanceStageBgm(bgmState).catch(() => {});
    };
    bgmAudioEl.load();
    bgmLoadedTrackId = trackId;
    await startBgmPlay(trackId, seq, targetVol, false);
  } catch (err) {
    if (seq === bgmLoadSeq) {
      bgmStatusText = "error:" + (err?.message || "加载失败");
      if (st.online) await advanceStageBgm(st, true).catch(() => {});
    }
  } finally {
    if (bgmLoadingTrackId === trackId) bgmLoadingTrackId = "";
  }
}

// Stage events
let sseAbort = null;
function handleStageEvent(msg) {
  if (!stageOwner || !msg?.event) return;
  switch (msg.event) {
    case "song_prepare":
      prepareSong(msg);
      break;
    case "song_play":
      playSong(msg);
      break;
    case "song_stop":
      stopSong();
      break;
    case "song_volume":
      if (songAudioEl && msg.volume != null) songAudioEl.volume = clampVol(msg.volume);
      break;
    case "bgm_state":
      if (msg.bgm) applyBgmState(msg.bgm);
      break;
  }
}

async function connectEvents() {
  if (!token) return;
  if (sseAbort) {
    sseAbort.abort();
    sseAbort = null;
  }
  const ctrl = new AbortController();
  sseAbort = ctrl;
  while (!ctrl.signal.aborted && token) {
    try {
      const r = await fetch("/api/stage/events", {
        headers: { Authorization: "Bearer " + token },
        signal: ctrl.signal,
        cache: "no-store",
      });
      if (!r.ok || !r.body) throw Error("sse failed");
      const reader = r.body.getReader();
      const decoder = new TextDecoder();
      let buf = "";
      while (!ctrl.signal.aborted) {
        const { value, done } = await reader.read();
        if (done) break;
        buf += decoder.decode(value, { stream: true });
        let idx;
        while ((idx = buf.indexOf("\n\n")) !== -1) {
          const block = buf.slice(0, idx);
          buf = buf.slice(idx + 2);
          for (const line of block.split("\n")) {
            if (line.startsWith("data:")) {
              try {
                handleStageEvent(JSON.parse(line.slice(5).trim()));
              } catch {}
            }
          }
        }
      }
    } catch {
      if (ctrl.signal.aborted) break;
    }
    await new Promise((res) => setTimeout(res, 1500));
  }
}

let serverBootId = "";
async function poll() {
  if (!token || pollBusy) return;
  pollBusy = true;
  try {
    observe();
    const reqBody = pack();
    forceTakeover = false;
    const r = await fetch("/api/output", {
      method: "POST",
      headers: { Authorization: "Bearer " + token, "Content-Type": "application/json" },
      body: JSON.stringify(reqBody),
    });
    if (!r.ok) throw Error("output rejected");
    const v = await r.json();
    if (v.boot_id) {
      if (serverBootId && serverBootId !== v.boot_id) {
        location.reload();
        return;
      }
      serverBootId = v.boot_id;
    }
    stageOwner = v.owner;
    dance.sync(stageOwner ? v.dance : null);
    if (!v.owner) {
      stopCheck();
      stopSong();
      stopBgm();
      if (rtc || desired) await leave();
      if (occupiedNotice) occupiedNotice.hidden = false;
      document.title = "Vorela · 舞台已被占用";
      return;
    }
    if (occupiedNotice) occupiedNotice.hidden = true;
    document.title = "Vorela · 舞台输出";
    if (v.bgm) applyBgmState(v.bgm);
    if (!v.song && (songPlaying || songPlaybackId)) {
      stopSong();
    } else if (v.song && v.song.state === "play" && (!songPlaying || songPlaybackId !== v.song.playback_id)) {
      playSong(v.song);
    } else if (v.song && v.song.state === "prepare" && songPlaybackId !== v.song.playback_id) {
      prepareSong(v.song);
    }
    if (!v.rtc || !v.live_id) {
      if (rtc || desired) await leave();
      probeReceiver();
      caption.textContent = "";
      audition(danceAudio ? "" : (v.check || ""));
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
  dance.sync(null);
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
  connectEvents();
}
if (takeoverBtn) {
  takeoverBtn.onclick = () => {
    forceTakeover = true;
    // Unlock audio elements in this tab inside the click gesture.
    for (const el of [bgmAudioEl, songAudioEl]) {
      try {
        el.play().catch(() => {});
        el.pause();
      } catch {}
    }
    poll();
  };
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
  sessionStorage.setItem("vorela.stage", next);
  history.replaceState(null, "", location.pathname);
  startPolling();
});
