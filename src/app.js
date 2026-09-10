// OTPBar 창 UI.
// 이 스크립트는 비밀키를 다루지 않는다. 백엔드에 요청하고 계정 이름과 코드만 받는다.

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const dialog = window.__TAURI__.dialog;
const autostart = window.__TAURI__.autostart;

const $ = (id) => document.getElementById(id);
const views = ["view-setup", "view-locked", "view-list", "view-add", "view-settings"];

let state = { unlocked: false, hasVault: false, settings: null, version: "" };
let reveal = false;          // 코드 보이기 토글(창을 닫으면 초기화)
let items = [];
let ticking = null;
let previousView = "view-list";

function show(view) {
  if (view !== "view-add" && view !== "view-settings") previousView = view;
  views.forEach((v) => ($(v).hidden = v !== view));
}

function setError(id, message) {
  const el = $(id);
  if (el) el.textContent = message || "";
}

async function refreshStatus() {
  state = await invoke("vault_status").then((s) => ({
    unlocked: s.unlocked,
    hasVault: s.has_vault,
    settings: s.settings,
    version: s.version,
    accountCount: s.account_count,
  }));
  if (!state.hasVault) {
    show("view-setup");
  } else if (!state.unlocked) {
    reveal = false;
    items = [];
    show("view-locked");
    $("unlock-pw").value = "";
    $("unlock-pw").focus();
  } else {
    show("view-list");
    await loadAccounts();
  }
  $("foot-status").textContent = `OTPBar ${state.version}`;
  $("about-text").textContent = `OTPBar ${state.version} · 비밀키는 마스터 암호로 암호화되어 이 기기에만 저장됩니다.`;
}

async function loadAccounts() {
  try {
    items = await invoke("list_accounts", { withCodes: true });
  } catch (e) {
    if (String(e).includes("잠")) return refreshStatus();
    items = [];
  }
  renderList();
}

function ringSvg(remaining, period) {
  const r = 10, c = 2 * Math.PI * r;
  const ratio = Math.max(0, Math.min(1, remaining / (period || 30)));
  return `<svg class="ring" viewBox="0 0 26 26" aria-hidden="true">
    <circle class="bg" cx="13" cy="13" r="${r}"></circle>
    <circle class="fg" cx="13" cy="13" r="${r}"
      stroke-dasharray="${c.toFixed(2)}" stroke-dashoffset="${(c * (1 - ratio)).toFixed(2)}"
      transform="rotate(-90 13 13)" stroke-linecap="round"></circle>
  </svg>`;
}

function formatCode(code) {
  if (!code) return "------";
  return code.length === 6 ? `${code.slice(0, 3)} ${code.slice(3)}` : code;
}

function maskCode(code) {
  return code && code.length === 6 ? "••• •••" : "•".repeat((code || "").length || 6);
}

function renderList() {
  const list = $("list");
  const masked = state.settings?.mask_codes && !reveal;
  $("btn-reveal").setAttribute("aria-pressed", String(reveal));
  $("btn-reveal").textContent = reveal ? "가리기" : "보기";
  if (items.length === 0) {
    list.innerHTML = `<div class="empty">
      등록된 계정이 없습니다.<br>
      <button id="btn-empty-add" class="link">QR 이미지에서 가져오기</button>
    </div>`;
    $("btn-empty-add").addEventListener("click", () => show("view-add"));
    return;
  }
  list.innerHTML = items
    .map((it) => {
      const a = it.account;
      const code = it.code || "";
      const remaining = it.remaining ?? 0;
      const expiring = remaining <= 5 ? " expiring" : "";
      return `<div class="row${expiring}" role="listitem" data-id="${escapeAttr(a.id)}" title="눌러서 복사">
        ${ringSvg(remaining, a.period)}
        <span class="meta">
          <span class="issuer">${escapeHtml(a.issuer || a.name)}</span>
          <span class="name">${escapeHtml(a.issuer ? a.name : "")}</span>
        </span>
        <span class="code${masked ? " masked" : ""}">${escapeHtml(masked ? maskCode(code) : formatCode(code))}</span>
        <button class="del" data-del="${escapeAttr(a.id)}" title="삭제">삭제</button>
      </div>`;
    })
    .join("");
}

function escapeHtml(s) {
  return String(s ?? "").replace(/[&<>"']/g, (c) =>
    ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
}
const escapeAttr = escapeHtml;

// 1초마다 남은 시간을 갱신하고, 코드가 바뀌는 순간 새로 받아 온다.
function startTicking() {
  if (ticking) clearInterval(ticking);
  ticking = setInterval(async () => {
    if (!state.unlocked || $("view-list").hidden) return;
    const before = items.map((i) => i.code).join(",");
    await loadAccounts();
    const after = items.map((i) => i.code).join(",");
    if (before !== after) renderList();
  }, 1000);
}

// ---------- 이벤트 ----------

$("form-setup").addEventListener("submit", async (e) => {
  e.preventDefault();
  setError("setup-error", "");
  const pw = $("setup-pw").value;
  if (pw !== $("setup-pw2").value) return setError("setup-error", "두 암호가 다릅니다.");
  try {
    await invoke("create_vault", { password: pw });
    $("setup-pw").value = $("setup-pw2").value = "";
    await refreshStatus();
  } catch (err) {
    setError("setup-error", String(err));
  }
});

$("form-unlock").addEventListener("submit", async (e) => {
  e.preventDefault();
  setError("unlock-error", "");
  try {
    await invoke("unlock", { password: $("unlock-pw").value });
    $("unlock-pw").value = "";
    await refreshStatus();
  } catch (err) {
    setError("unlock-error", String(err));
  }
});

$("btn-lock").addEventListener("click", async () => {
  await invoke("lock");
  await refreshStatus();
});

$("btn-reveal").addEventListener("click", () => {
  reveal = !reveal;
  renderList();
});

$("list").addEventListener("click", async (e) => {
  const del = e.target.closest("[data-del]");
  if (del) {
    e.stopPropagation();
    const id = del.getAttribute("data-del");
    const ok = await dialog.confirm(`${id} 계정을 삭제할까요? 되돌릴 수 없습니다.`, { title: "계정 삭제", kind: "warning" });
    if (ok) {
      await invoke("remove_account", { query: id });
      await loadAccounts();
    }
    return;
  }
  const row = e.target.closest(".row");
  if (!row) return;
  try {
    await invoke("get_code", { query: row.getAttribute("data-id"), copy: true });
    row.classList.add("copied");
    setTimeout(() => row.classList.remove("copied"), 600);
  } catch (err) {
    await dialog.message(String(err), { title: "코드를 가져오지 못했습니다", kind: "error" });
  }
});

$("btn-add").addEventListener("click", () => show("view-add"));
$("btn-settings").addEventListener("click", async () => {
  await fillSettings();
  show("view-settings");
});
document.querySelectorAll("[data-back]").forEach((b) =>
  b.addEventListener("click", () => {
    setError("add-error", "");
    $("add-log").hidden = true;
    show(previousView);
  })
);

$("btn-pick-qr").addEventListener("click", async () => {
  const picked = await dialog.open({
    multiple: true,
    filters: [{ name: "이미지", extensions: ["png", "jpg", "jpeg", "bmp", "webp", "tiff"] }],
  });
  if (!picked) return;
  const paths = Array.isArray(picked) ? picked : [picked];
  const log = [];
  for (const p of paths) {
    try {
      const res = await invoke("import_source", { source: p, replace: false });
      log.push(...res.messages);
    } catch (err) {
      log.push(`실패: ${p} — ${err}`);
    }
  }
  showImportLog(log);
});

$("form-uri").addEventListener("submit", async (e) => {
  e.preventDefault();
  const uri = $("uri").value.trim();
  if (!uri) return;
  try {
    const res = await invoke("import_source", { source: uri, replace: false });
    $("uri").value = "";
    showImportLog(res.messages);
  } catch (err) {
    setError("add-error", String(err));
  }
});

$("form-manual").addEventListener("submit", async (e) => {
  e.preventDefault();
  setError("add-error", "");
  try {
    await invoke("add_account", {
      issuer: $("m-issuer").value.trim(),
      name: $("m-name").value.trim(),
      secret: $("m-secret").value.trim(),
      digits: Number($("m-digits").value) || 6,
      period: Number($("m-period").value) || 30,
      algorithm: $("m-algo").value,
      replace: false,
    });
    $("m-secret").value = "";
    showImportLog(["등록했습니다."]);
  } catch (err) {
    setError("add-error", String(err));
  }
});

function showImportLog(lines) {
  const log = $("add-log");
  log.textContent = lines.join("\n");
  log.hidden = lines.length === 0;
  loadAccounts();
}

async function fillSettings() {
  const s = state.settings || (await invoke("get_settings"));
  $("s-mask").checked = !!s.mask_codes;
  $("s-notify-code").checked = !!s.show_code_in_notification;
  $("s-autolock").value = Math.round((s.auto_lock_secs || 0) / 60);
  $("s-clip").value = s.clipboard_clear_secs ?? 20;
  $("about-dir").textContent = await invoke("config_dir");
  try {
    $("s-autostart").checked = await autostart.isEnabled();
  } catch {
    $("s-autostart").disabled = true;
  }
}

$("btn-save-settings").addEventListener("click", async () => {
  setError("settings-error", "");
  $("settings-ok").textContent = "";
  try {
    const settings = {
      auto_lock_secs: Math.max(0, Number($("s-autolock").value) || 0) * 60,
      mask_codes: $("s-mask").checked,
      show_code_in_notification: $("s-notify-code").checked,
      clipboard_clear_secs: Math.max(0, Number($("s-clip").value) || 0),
      auto_update_check: true,
    };
    await invoke("set_settings", { settings });
    state.settings = settings;
    try {
      if ($("s-autostart").checked) await autostart.enable();
      else await autostart.disable();
    } catch { /* 자동 시작을 못 바꿔도 설정 저장은 유지 */ }
    $("settings-ok").textContent = "저장했습니다.";
    renderList();
  } catch (err) {
    setError("settings-error", String(err));
  }
});

$("form-chpw").addEventListener("submit", async (e) => {
  e.preventDefault();
  setError("settings-error", "");
  $("settings-ok").textContent = "";
  try {
    await invoke("change_password", { current: $("cp-current").value, newPassword: $("cp-new").value });
    $("cp-current").value = $("cp-new").value = "";
    $("settings-ok").textContent = "마스터 암호를 바꿨습니다.";
  } catch (err) {
    setError("settings-error", String(err));
  }
});

$("btn-update").addEventListener("click", checkUpdate);

async function checkUpdate() {
  $("foot-status").textContent = "업데이트 확인 중…";
  try {
    const version = await invoke("check_update");
    if (!version) {
      $("foot-status").textContent = `최신 상태입니다 (${state.version})`;
      return;
    }
    const ok = await dialog.confirm(`새 버전 ${version}이 있습니다. 지금 설치할까요?`, { title: "OTPBar 업데이트" });
    if (!ok) {
      $("foot-status").textContent = `업데이트 ${version} 대기 중`;
      return;
    }
    $("foot-status").textContent = "설치 중…";
    await invoke("install_update");
    await dialog.message("설치했습니다. 앱을 다시 시작하면 새 버전으로 실행됩니다.", { title: "OTPBar" });
  } catch (err) {
    const raw = String(err);
    const friendly = /valid release JSON|404|Not Found/i.test(raw)
      ? "아직 배포된 업데이트가 없습니다."
      : /network|dns|timed out|connect/i.test(raw)
        ? "네트워크에 연결하지 못했습니다."
        : `업데이트 확인 실패: ${raw}`;
    $("foot-status").textContent = friendly;
  }
}

listen("otpbar://refresh", refreshStatus);
listen("otpbar://import-dialog", () => show("view-add"));
listen("otpbar://check-update", checkUpdate);

window.addEventListener("blur", () => {
  // 창에서 포커스가 벗어나면 코드 표시를 다시 가린다
  if (reveal) {
    reveal = false;
    renderList();
  }
});

refreshStatus().then(startTicking);
