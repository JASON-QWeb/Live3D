import { action, api, confirmAction, el, notify, request } from "./settings.js";
import { t } from "./i18n.js";

const select = el("danceSelect"), upload = el("danceImport"), status = el("danceStatus");
let state, importing = false;

export function paintDance(next) {
  state = next;
  const dance = state?.dance;
  el("dancePlay").disabled = !state?.output || !select.value || dance?.clip.id === select.value;
  el("danceRemove").disabled = !select.value || dance?.clip.id === select.value;
  el("danceStop").hidden = !dance;
  if (!importing) {
    status.textContent = dance ? t(dance.started ? "正在播放 · {0}" : "等待舞台播放 · {0}", dance.clip.title)
      : select.value && !state?.output ? t("请先打开舞台") : "";
  }
}

export async function loadDances(preferred = select.value) {
  const clips = await api("/api/dances");
  select.replaceChildren();
  if (!clips.length) select.add(new Option(t("暂无舞蹈视频"), ""));
  for (const clip of clips) {
    const seconds = Math.floor(clip.duration);
    const duration = Math.floor(seconds / 60) + ":" + String(seconds % 60).padStart(2, "0");
    select.add(new Option(clip.title + " · " + duration, clip.id));
  }
  if (clips.some(c => c.id === preferred)) select.value = preferred;
  paintDance(state);
}
select.onchange = () => paintDance(state);
upload.onchange = async () => {
  importing = upload.disabled = true;
  let last;
  try {
    for (const file of Array.from(upload.files)) {
      status.textContent = t("正在导入 · {0}", file.name);
      const r = await request("/api/dances/import/" + encodeURIComponent(file.name), { method: "PUT", body: file });
      last = (await r.json()).id;
    }
  } catch (error) {
    notify(error.message);
  } finally {
    upload.value = "";
    importing = upload.disabled = false;
    await loadDances(last).catch(e => notify(e.message));
  }
};

action("dancePlay", async () => {
  await api("/api/dances/" + select.value, {});
  paintDance(await api("/api/status"));
});
action("danceStop", async () => {
  if (state?.dance) await api("/api/dances/stop/" + state.dance.id, {});
  paintDance(await api("/api/status"));
});
action("danceRemove", async () => {
  const id = select.value;
  if (await confirmAction(t("移除舞蹈视频"), select.selectedOptions[0].text, t("移除"))) {
    await request("/api/dances/" + id, { method: "DELETE" });
    await loadDances();
  }
});
