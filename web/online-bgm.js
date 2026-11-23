import { action, api, el, notify, confirmAction, request } from "./settings.js";
import { t } from "./i18n.js";

export async function loadBgm(timespan) {
  const items = await api("/api/bgm"),
    select = el("bgmTracks"),
    old = select.value;
  select.replaceChildren();
  if (!items.length) select.add(new Option(t("暂无背景音乐"), ""));
  for (const t of items) {
    select.add(new Option(t.title + " · " + timespan(t.duration_ms), t.id));
  }
  if (items.some((t) => t.id === old)) select.value = old;
  el("bgmCount").textContent = items.length ? t("共 {0} 首", items.length) : "";
  el("toggleBgm").disabled = !items.length;
  el("nextBgm").disabled = items.length < 2;
  el("previewBgm").disabled = !items.length;
  el("removeBgm").disabled = !items.length;
  el("clearBgm").disabled = !items.length;
}

function setupMusicAccount(provider) {
  const uin = el("musicUin"), key = el("musicKey");
  const status = el("musicAccountState"), panel = el("musicAccount");
  const remove = el("clearMusicCookie"), save = el("saveMusicCookie");
  let revision = 0;
  async function sync(method = "GET") {
    const current = ++revision;
    if (provider.value !== "qq") return;
    let payload;
    if (method === "PUT") {
      const id = uin.value.trim(), secret = key.value.trim();
      if (!/^o*\d+$/.test(id)) throw new Error(t("请输入有效的 uin"));
      if (!secret || /[;\s]/.test(secret)) throw new Error(t("请输入 qqmusic_key 的值"));
      payload = { cookie: `uin=${id}; qqmusic_key=${secret}` };
    }
    if (method !== "GET") {
      provider.disabled = true;
      uin.disabled = key.disabled = true;
      save.disabled = true;
      remove.disabled = true;
    }
    try {
      const result = await api("/api/bgm/online/account/qq", payload, method);
      if (current !== revision) return;
      if (method !== "GET") uin.value = key.value = "";
      status.textContent = t(result.configured ? "已保存 Cookie" : "尚未配置 Cookie");
      remove.hidden = !result.configured;
    } finally {
      if (method !== "GET") {
        provider.disabled = false;
        uin.disabled = key.disabled = false;
        save.disabled = false;
        remove.disabled = false;
      }
    }
  }
  function refresh() {
    ++revision;
    panel.hidden = provider.value !== "qq";
    uin.value = key.value = "";
    status.textContent = "";
    remove.hidden = true;
    if (!panel.hidden && panel.open) sync().catch(e => notify(e.message));
  }
  panel.addEventListener("toggle", refresh);
  provider.addEventListener("change", refresh);
  refresh();
  action("saveMusicCookie", () => sync("PUT"));
  action("clearMusicCookie", () => sync("DELETE"));
}

export function setupOnlineBgm(loadBgm, tick) {
  action("nextBgm", async () => {
    await api("/api/bgm/control", { action: "next" });
    await tick();
  });

  action("removeBgm", async () => {
    if (await confirmAction(t("移除背景音乐"), t("移除所选背景音乐在 Vorela 中的副本。"), t("移除"))) {
      await request("/api/bgm/" + el("bgmTracks").value, { method: "DELETE" });
      await loadBgm();
      await tick();
    }
  });

  action("clearBgm", async () => {
    if (await confirmAction(t("清空背景音乐"), t("移除全部背景音乐副本（不影响原始文件）。"), t("清空全部"))) {
      await request("/api/bgm", { method: "DELETE" });
      await loadBgm();
      await tick();
      notify(t("已清空背景音乐"));
    }
  });

  const provider = el("musicProvider"), term = el("musicTerm"), results = el("musicResults");
  setupMusicAccount(provider);
  let matches = [];
  const clear = () => {
    matches = [];
    results.replaceChildren();
    el("musicMatches").hidden = true;
  };
  term.addEventListener("input", clear);
  provider.addEventListener("change", clear);
  action("searchMusic", async () => {
    clear();
    const input = term.value.trim(), platform = provider.value;
    const found = await api("/api/bgm/online/search", { provider: platform, input });
    if (input !== term.value.trim() || platform !== provider.value) return;
    matches = found;
    for (const [index, track] of matches.entries()) results.add(new Option(track.title, index));
    el("musicMatches").hidden = !matches.length;
    if (!matches.length) notify(t("没有找到匹配歌曲"));
  });
  action("requestMusic", async () => {
    const track = matches[results.selectedIndex];
    if (!track) return;
    await api("/api/bgm/online/request", track);
    notify(t("已点播 · {0}", track.title));
    await tick();
  });
  action("importPlaylist", async () => {
    const result = await api("/api/bgm/online/import", { provider: provider.value, input: el("musicPlaylist").value.trim() });
    await loadBgm();
    await tick();
    notify(t("已添加 {0} 首背景音乐", result.added));
  });
}

export function paintOnlineBgm(state) {
  const available = Boolean(el("bgmTracks").value);
  el("toggleBgm").disabled = !available && !state.bgm_requested;
  el("nextBgm").disabled = el("bgmTracks").options.length < 2 && !state.bgm_requested && !state.bgm_requests.length;
  el("bgmQueue").textContent = state.bgm_requests.length ? t("待播 · {0}", state.bgm_requests.join(" → ")) : "";
}
