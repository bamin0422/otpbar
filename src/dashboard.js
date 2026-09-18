// 대시보드 동작. 창을 다시 들이지 않으려고 프레임워크 없이 쓴다.

const invoke = window.__TAURI__.core.invoke;

const list = document.getElementById("list");
const empty = document.getElementById("empty");
const noMatch = document.getElementById("no-match");
const locked = document.getElementById("locked");
const status = document.getElementById("status");
const search = document.getElementById("search");
const maskBtn = document.getElementById("mask");
const retryBtn = document.getElementById("retry");

// 트레이 등록 실패로 열렸을 때만 복구 안내를 보여 준다.
const trayFailed = new URLSearchParams(location.search).get("tray") === "failed";
if (trayFailed) {
  document.getElementById("tray-notice").hidden = false;
  retryBtn.hidden = false;
}

/// 원의 둘레. r=9 인 원에 맞춘다.
const RING = 2 * Math.PI * 9;

let query = "";
/// 지금 그려져 있는 행의 id 순서. 이것이 같으면 값만 갱신한다.
let drawn = [];

function say(text) {
  status.textContent = text;
}

function pretty(code) {
  return code.length === 6 ? `${code.slice(0, 3)} ${code.slice(3)}` : code;
}

function shown(item, masked) {
  if (!item.code) return "------";
  if (masked) return item.code.length === 6 ? "••• •••" : "•".repeat(item.code.length);
  return pretty(item.code);
}

function matches(item) {
  if (!query) return true;
  const q = query.toLowerCase();
  return (
    item.issuer.toLowerCase().includes(q) || item.name.toLowerCase().includes(q)
  );
}

function ring(item) {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("class", "ring");
  svg.setAttribute("viewBox", "0 0 22 22");
  for (const cls of ["ring-track", "ring-value"]) {
    const c = document.createElementNS("http://www.w3.org/2000/svg", "circle");
    c.setAttribute("class", cls);
    c.setAttribute("cx", "11");
    c.setAttribute("cy", "11");
    c.setAttribute("r", "9");
    if (cls === "ring-value") c.setAttribute("stroke-dasharray", String(RING));
    svg.appendChild(c);
  }
  setRing(svg, item);
  return svg;
}

function setRing(svg, item) {
  const value = svg.querySelector(".ring-value");
  if (!value) return;
  const period = item.period || 30;
  const left = item.remaining != null ? item.remaining : 0;
  // 남은 비율만큼 원을 채운다.
  value.setAttribute("stroke-dashoffset", String(RING * (1 - left / period)));
}

function row(item, masked) {
  const li = document.createElement("li");
  li.dataset.id = item.id;

  const label = document.createElement("span");
  label.className = "label";
  label.textContent = `${item.issuer} · ${item.name}`;

  const code = document.createElement("span");
  code.className = "code";
  code.textContent = shown(item, masked);

  li.append(label, code, ring(item));
  li.addEventListener("click", () => copy(item.id, li));
  return li;
}

async function copy(id, li) {
  try {
    say(await invoke("dashboard_copy", { id }));
    li.classList.add("copied");
    setTimeout(() => li.classList.remove("copied"), 1200);
  } catch (e) {
    say(`복사하지 못했습니다: ${e}`);
  }
}

async function refresh() {
  let view;
  try {
    view = await invoke("dashboard_list");
  } catch (e) {
    say(`목록을 읽지 못했습니다: ${e}`);
    return;
  }

  locked.hidden = view.unlocked;
  maskBtn.textContent = view.masked ? "코드 보이기" : "코드 가리기";

  const visible = view.items.filter(matches);
  empty.hidden = !view.unlocked || view.items.length > 0;
  noMatch.hidden = !view.unlocked || view.items.length === 0 || visible.length > 0;

  const ids = visible.map((i) => i.id);
  const same =
    ids.length === drawn.length && ids.every((id, i) => id === drawn[i]);

  if (same) {
    visible.forEach((item, i) => {
      const li = list.children[i];
      li.querySelector(".code").textContent = shown(item, view.masked);
      setRing(li.querySelector(".ring"), item);
    });
    return;
  }

  list.replaceChildren(...visible.map((item) => row(item, view.masked)));
  drawn = ids;
}

search.addEventListener("input", () => {
  query = search.value.trim();
  refresh();
});

maskBtn.addEventListener("click", async () => {
  await invoke("dashboard_toggle_mask");
  refresh();
});

document.getElementById("unlock").addEventListener("click", async () => {
  await invoke("dashboard_unlock");
  say("잠금 해제를 요청했습니다.");
});

retryBtn.addEventListener("click", async () => {
  say("트레이를 다시 만드는 중입니다…");
  try {
    const ok = await invoke("dashboard_retry_tray");
    say(
      ok
        ? "트레이를 다시 만들었습니다. 작업 표시줄을 확인하십시오."
        : "이번에도 실패했습니다. 이 창으로 계속 쓰실 수 있습니다."
    );
  } catch (e) {
    say(`재시도하지 못했습니다: ${e}`);
  }
});

document.getElementById("update").addEventListener("click", async () => {
  await invoke("dashboard_check_update");
  say("업데이트를 확인하는 중입니다. 결과는 알림으로 알려 드립니다.");
});

refresh();
setInterval(refresh, 1000);
