'use strict';

// ── Auth state (in-memory only, never localStorage) ────────
let jwt = null;

// Demo credential store — replace with POST /api/auth/login
const users = {};

// ── View switching ─────────────────────────────────────────

function showAuth() {
  document.getElementById('auth-view').classList.remove('hidden');
  document.getElementById('dashboard-view').classList.add('hidden');
}

function showDashboard() {
  document.getElementById('auth-view').classList.add('hidden');
  document.getElementById('dashboard-view').classList.remove('hidden');
}

// ── Tab switching ──────────────────────────────────────────

function switchTab(tab) {
  document.getElementById('tab-login').classList.toggle('active', tab === 'login');
  document.getElementById('tab-signup').classList.toggle('active', tab === 'signup');
  document.getElementById('form-login').classList.toggle('hidden', tab !== 'login');
  document.getElementById('form-signup').classList.toggle('hidden', tab !== 'signup');
  document.getElementById('login-error').textContent = '';
  document.getElementById('signup-error').textContent = '';
}

// ── Auth actions ───────────────────────────────────────────

function doLogin() {
  const user = document.getElementById('login-user').value.trim();
  const pass = document.getElementById('login-pass').value;
  const err  = document.getElementById('login-error');

  if (!user || !pass)               { err.textContent = 'Username and password required'; return; }
  if (!users[user.toLowerCase()])   { err.textContent = 'Invalid credentials'; return; }
  if (users[user.toLowerCase()] !== pass) { err.textContent = 'Invalid credentials'; return; }

  err.textContent = '';
  jwt = 'demo-jwt-' + Date.now();
  showDashboard();
}

function doSignup() {
  const user    = document.getElementById('signup-user').value.trim();
  const pass    = document.getElementById('signup-pass').value;
  const confirm = document.getElementById('signup-confirm').value;
  const err     = document.getElementById('signup-error');

  if (!user || !pass)               { err.textContent = 'All fields required'; return; }
  if (pass.length < 12)             { err.textContent = 'Password must be at least 12 characters'; return; }
  if (pass !== confirm)             { err.textContent = 'Passwords do not match'; return; }
  if (users[user.toLowerCase()])    { err.textContent = 'Username already taken'; return; }

  err.textContent = '';
  users[user.toLowerCase()] = pass;
  jwt = 'demo-jwt-' + Date.now();
  showDashboard();
}

function doLogout() {
  jwt = null;
  document.getElementById('login-user').value    = '';
  document.getElementById('login-pass').value    = '';
  document.getElementById('login-error').textContent = '';
  switchTab('login');
  showAuth();
}

// Enter key support on auth forms
document.addEventListener('keydown', function (e) {
  if (e.key !== 'Enter') return;
  if (!document.getElementById('auth-view').classList.contains('hidden') &&
      document.getElementById('auth-view').style.display !== 'none') {
    const loginActive = !document.getElementById('form-login').classList.contains('hidden');
    loginActive ? doLogin() : doSignup();
  }
});

// ── Dashboard — bot state ──────────────────────────────────

let running = true;
let block   = 21847392;

function toggleBot() {
  running = !running;
  const dot   = document.getElementById('status-dot');
  const label = document.getElementById('status-label');
  const btn   = document.getElementById('toggle-btn');

  if (running) {
    dot.className     = 'status-dot';
    label.textContent = 'RUNNING';
    btn.textContent   = 'STOP BOT';
    btn.className     = 'btn btn-stop';
  } else {
    dot.className     = 'status-dot stopped';
    label.textContent = 'STOPPED';
    btn.textContent   = 'START BOT';
    btn.className     = 'btn btn-start';
  }
}

// ── Dashboard — mock data ──────────────────────────────────

const routes = [
  'WETH → USDC → WETH',
  'WETH → DAI  → USDC',
  'USDC → WETH → DAI',
  'WETH → USDT → WETH',
  'DAI  → WETH → USDC',
];

function randHash() {
  const h = '0123456789abcdef';
  let s = '0x';
  for (let i = 0; i < 16; i++) s += h[Math.floor(Math.random() * 16)];
  return s + '...';
}

function randGas()          { return '$' + (Math.random() * 12 + 3).toFixed(2); }
function randProfit(status) { return status === 'failed' ? '-' + randGas() : '+$' + (Math.random() * 80 + 5).toFixed(2); }
function timeNow()          { return new Date().toLocaleTimeString('en-US', { hour12: false }); }

const transactions = [];

function addTx() {
  if (!running) return;
  const statuses = ['success', 'success', 'success', 'failed', 'success'];
  const status   = statuses[Math.floor(Math.random() * statuses.length)];
  const profit   = randProfit(status);
  transactions.unshift({
    time:  timeNow(),
    hash:  randHash(),
    route: routes[Math.floor(Math.random() * routes.length)],
    gas:   randGas(),
    profit,
    status,
  });
  if (transactions.length > 50) transactions.pop();
  renderTx();
}

function renderTx() {
  document.getElementById('tx-body').innerHTML = transactions.map(tx => `
    <tr>
      <td>${tx.time}</td>
      <td><a class="tx-hash" href="#" onclick="return false">${tx.hash}</a></td>
      <td>${tx.route}</td>
      <td>${tx.gas}</td>
      <td class="${tx.profit.startsWith('+') ? 'profit-pos' : 'profit-neg'}">${tx.profit}</td>
      <td><span class="tag tag-${tx.status}">${tx.status}</span></td>
    </tr>
  `).join('');
}

function tickBlock() {
  if (!running) return;
  block += 1;
  document.getElementById('block-num').textContent = block.toLocaleString();
}

function updateGas() {
  const base     = (Math.random() * 8 + 8).toFixed(1);
  const priority = (Math.random() * 1 + 1).toFixed(1);
  const gate     = (parseFloat(base) * 0.3 + 1.5).toFixed(2);
  document.getElementById('base-fee').textContent     = base + ' gwei';
  document.getElementById('priority-fee').textContent = priority + ' gwei';
  document.getElementById('min-gate').textContent     = '$' + gate;
}

// ── Init ───────────────────────────────────────────────────

for (let i = 0; i < 20; i++) addTx();
updateGas();

setInterval(tickBlock, 12000);
setInterval(addTx,     4000);
setInterval(updateGas, 6000);
