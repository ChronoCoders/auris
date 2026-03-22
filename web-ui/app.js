let token = null;

function setError(msg) {
  const el = document.getElementById("auth-error");
  el.textContent = msg || "";
}

function showDashboard() {
  document.getElementById("auth").classList.add("hidden");
  document.getElementById("dashboard").classList.remove("hidden");
}

async function login(username, password) {
  const resp = await fetch("/api/auth/login", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ username, password }),
  });
  const body = await resp.json().catch(() => null);
  if (!resp.ok) {
    setError(body && body.error ? body.error : "Login failed");
    return;
  }
  token = body.token;
  showDashboard();
  await refreshStatus();
}

async function refreshStatus() {
  const resp = await fetch("/api/bot/status", {
    headers: { Authorization: `Bearer ${token}` },
  });
  const body = await resp.json().catch(() => null);
  const el = document.getElementById("status");
  if (!resp.ok) {
    el.textContent = body && body.error ? body.error : "Failed to load status";
    return;
  }
  el.textContent = `Bot status: ${body.status} | block: ${body.current_block}`;
}

document.getElementById("login-form").addEventListener("submit", (e) => {
  e.preventDefault();
  setError("");
  const username = document.getElementById("login-username").value.trim();
  const password = document.getElementById("login-password").value;
  login(username, password).catch((err) => setError(String(err)));
});
