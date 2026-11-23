function sendReceipt(client, token, id, state) {
  return fetch("/api/stage/dance/" + id, {
    method: "POST", headers: { Authorization: "Bearer " + token, "Content-Type": "application/json" },
    body: JSON.stringify({ client, state }),
  });
}

export function dancePlayer(client, token, setAudio) {
  const layer = document.getElementById("danceLayer"), unlock = document.getElementById("danceUnlock");
  let current = null;
  function clear() {
    if (!current) return;
    const { video } = current;
    current = null;
    video.pause();
    video.removeAttribute("src");
    video.load();
    video.remove();
    unlock.hidden = true;
    setAudio(false);
  }
  async function report(item, state) {
    if (current !== item || item.reporting) return;
    item.reporting = true;
    try {
      const response = await sendReceipt(client, token(), item.id, state);
      if (response.ok && state === "playing") item.ack = true;
    } catch {} finally {
      item.reporting = false;
    }
  }
  function finish(item, state) {
    if (current !== item) return;
    item.finished = state;
    item.video.hidden = true;
    unlock.hidden = true;
    setAudio(false);
    report(item, state);
  }
  function start(item) {
    item.video.play().catch(e => {
      if (current !== item) return;
      if (e.name === "NotAllowedError") unlock.hidden = false;
      else if (e.name !== "AbortError") finish(item, "error");
    });
  }
  unlock.onclick = () => { if (current) start(current); };
  return {
    visible: () => current && !current.video.hidden ? current : null,
    sync(dance) {
      if (current && current.id === dance?.id) {
        if (current.finished) report(current, current.finished);
        else if (!current.ack && !current.video.hidden) report(current, "playing");
        return;
      }
      clear();
      if (!dance) return;
      const video = document.createElement("video");
      const item = { id: dance.id, video };
      current = item;
      video.hidden = true;
      video.playsInline = true;
      video.onloadedmetadata = () => {
        if (current !== item) return;
        if (dance.started) {
          const elapsed = Math.max(0, (Date.now() - dance.started) / 1000);
          if (elapsed >= video.duration) { finish(item, "ended"); return; }
          video.currentTime = elapsed;
        }
        start(item);
      };
      video.onplaying = () => {
        if (current !== item || item.finished) return;
        video.hidden = false;
        unlock.hidden = true;
        setAudio(true);
        report(item, "playing");
      };
      video.onended = () => finish(item, "ended");
      video.onerror = () => finish(item, "error");
      layer.append(video);
      video.src = "/dance-video/" + dance.id;
    }
  };
}
