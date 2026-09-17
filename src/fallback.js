// 대체 창 동작. 트레이를 못 만들었을 때만 열리는 화면이다.
// 창을 다시 들이지 않으려고 프레임워크 없이 쓴다.

const invoke = window.__TAURI__.core.invoke;

const list = document.getElementById("list");
const empty = document.getElementById("empty");
const locked = document.getElementById("locked");
const status = document.getElementById("status");

function say(text) {
  status.textContent = text;
}

function pretty(code) {
  return code.length === 6 ? `${code.slice(0, 3)} ${code.slice(3)}` : code;
}

function row(item) {
  const li = document.createElement("li");
  li.dataset.id = item.id;

  const label = document.createElement("span");
  label.className = "label";
  label.textContent = `${item.issuer} · ${item.name}`;

  const code = document.createElement("span");
  code.className = "code";
  code.textContent = item.code ? pretty(item.code) : "------";

  const remaining = document.createElement("span");
  remaining.className = "remaining";
  remaining.textContent = item.remaining != null ? `${item.remaining}초` : "";

  li.append(label, code, remaining);
  li.addEventListener("click", () => copy(item.id));
  return li;
}

async function copy(id) {
  try {
    say(await invoke("fallback_copy", { id }));
  } catch (e) {
    say(`복사하지 못했습니다: ${e}`);
  }
}

async function refresh() {
  let view;
  try {
    view = await invoke("fallback_list");
  } catch (e) {
    say(`목록을 읽지 못했습니다: ${e}`);
    return;
  }

  locked.hidden = view.unlocked;
  empty.hidden = !view.unlocked || view.items.length > 0;

  // 항목 수가 그대로면 글자만 바꾼다. 매초 다시 그리면 클릭이 튄다.
  if (list.children.length === view.items.length) {
    view.items.forEach((item, i) => {
      const li = list.children[i];
      if (li.dataset.id !== item.id) {
        list.replaceChild(row(item), li);
        return;
      }
      li.querySelector(".code").textContent = item.code ? pretty(item.code) : "------";
      li.querySelector(".remaining").textContent =
        item.remaining != null ? `${item.remaining}초` : "";
    });
    return;
  }

  list.replaceChildren(...view.items.map(row));
}

document.getElementById("unlock").addEventListener("click", async () => {
  await invoke("fallback_unlock");
  say("잠금 해제를 요청했습니다.");
});

document.getElementById("retry").addEventListener("click", async () => {
  say("트레이를 다시 만드는 중입니다…");
  try {
    const ok = await invoke("fallback_retry_tray");
    say(ok
      ? "트레이를 다시 만들었습니다. 작업 표시줄을 확인하십시오. 보이면 이 창을 닫아도 됩니다."
      : "이번에도 실패했습니다. 이 창으로 계속 쓰실 수 있습니다.");
  } catch (e) {
    say(`재시도하지 못했습니다: ${e}`);
  }
});

document.getElementById("update").addEventListener("click", async () => {
  await invoke("fallback_check_update");
  say("업데이트를 확인하는 중입니다. 결과는 알림으로 알려 드립니다.");
});

refresh();
setInterval(refresh, 1000);
