'use strict';

/* =========================================================================
   星途车联 TSP · 运营控制台
   Vanilla JS, zero dependencies. REST + WebSocket.
   ========================================================================= */

const $ = (selector, root = document) => root.querySelector(selector);
const $$ = (selector, root = document) => Array.from(root.querySelectorAll(selector));

const STATUS = {
  moving: { label: '行驶中', color: '#2dd4bf', className: 'status-moving', icon: 'car' },
  idle: { label: '怠速在线', color: '#6fa6ff', className: 'status-idle', icon: 'car' },
  charging: { label: '充电中', color: '#f6b34b', className: 'status-charging', icon: 'bolt' },
  online: { label: '在线', color: '#7fb0ff', className: 'status-online', icon: 'wifi' },
  offline: { label: '离线', color: '#667d92', className: 'status-offline', icon: 'wifi' },
  maintenance: { label: '维保中', color: '#a78bfa', className: 'status-maintenance', icon: 'shield' }
};

const LEVEL = {
  critical: { label: '严重', badge: 'badge-critical', icon: 'alert' },
  warning: { label: '警告', badge: 'badge-warning', icon: 'alert' },
  info: { label: '提示', badge: 'badge-info', icon: 'info' }
};

const ALERT_META = {
  low_soc: { label: '低电量', icon: 'battery' },
  overspeed: { label: '超速', icon: 'gauge' },
  geofence_enter: { label: '进入围栏', icon: 'fence' },
  geofence_exit: { label: '驶出围栏', icon: 'fence' },
  collision: { label: '碰撞', icon: 'alert' },
  device_offline: { label: '终端离线', icon: 'wifi' },
  harsh_brake: { label: '急减速', icon: 'alert' },
  tire_pressure: { label: '胎压异常', icon: 'alert' },
  charge_complete: { label: '充电完成', icon: 'bolt' }
};

const COMMANDS = [
  { key: 'lock', label: '上锁', icon: 'lock' },
  { key: 'unlock', label: '解锁', icon: 'unlock' },
  { key: 'start', label: '远程启动', icon: 'power' },
  { key: 'stop', label: '安全熄火', icon: 'power', danger: true },
  { key: 'ac_on', label: '开启空调', icon: 'snow' },
  { key: 'ac_off', label: '关闭空调', icon: 'snow' },
  { key: 'flash', label: '闪灯', icon: 'locate' },
  { key: 'honk', label: '鸣笛', icon: 'horn' },
  { key: 'find', label: '寻车', icon: 'pin' },
  { key: 'charge_start', label: '开始充电', icon: 'bolt' },
  { key: 'charge_stop', label: '停止充电', icon: 'battery' },
  { key: 'window_close', label: '关闭车窗', icon: 'window' }
];

const COMMAND_LABEL = Object.fromEntries(COMMANDS.map((item) => [item.key, item]));
COMMAND_LABEL.stop = { key: 'stop', label: '安全熄火', icon: 'power' };

const VIEW_META = {
  dashboard: ['运营总览', '运营总览'],
  live: ['实时监控', '实时监控'],
  vehicles: ['车辆管理', '车辆管理'],
  trips: ['行程分析', '行程分析'],
  geofences: ['电子围栏', '电子围栏'],
  alerts: ['告警中心', '告警中心'],
  commands: ['指令中心', '指令中心'],
  terminals: ['终端管理', '终端管理'],
  media: ['媒体播放', 'JT/T 1078 媒体']
};

const state = {
  token: localStorage.getItem('tsp_token') || '',
  refreshToken: localStorage.getItem('tsp_refresh_token') || '',
  user: JSON.parse(localStorage.getItem('tsp_user') || 'null'),
  view: 'dashboard',
  dashboard: null,
  vehicles: [],
  allVehicles: [],
  geofences: [],
  alerts: null,
  commands: null,
  terminals: [],
  selectedVehicleId: null,
  selectedVehicle: null,
  liveTrajectory: [],
  liveAlerts: [],
  liveFilter: 'all',
  liveSearch: '',
  vehicleSearch: '',
  vehicleStatus: '',
  vehiclePage: 1,
  vehiclePageSize: 10,
  alertPage: 1,
  alertFilter: 'active',
  alertLevel: '',
  tripPage: 1,
  commandVehicleId: null,
  commandType: 'lock',
  map: null,
  modalMap: null,
  modalResolver: null,
  ws: null,
  wsReady: false,
  wsManualClose: false,
  reconnectTimer: null,
  clockTimer: null,
  refreshTimer: null,
  authRefreshTimer: null,
  alertUnread: 0
};

const canWrite = () => !!state.user && ['admin', 'operator'].includes(state.user.role);
const isAdmin = () => !!state.user && state.user.role === 'admin';

/* ------------------------------------------------------------------ utils */

function icon(name, className = '') {
  return `<svg class="icon ${className}"><use href="#i-${name}"></use></svg>`;
}

function escapeHtml(value) {
  return String(value ?? '')
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#039;');
}

function fmtNumber(value, digits = 0) {
  const number = Number(value);
  if (!Number.isFinite(number)) return '—';
  return number.toLocaleString('zh-CN', { minimumFractionDigits: digits, maximumFractionDigits: digits });
}

function fmtKm(value, digits = 1) {
  const number = Number(value);
  if (!Number.isFinite(number)) return '—';
  return `${number.toLocaleString('zh-CN', { minimumFractionDigits: digits, maximumFractionDigits: digits })}`;
}

function fmtDate(value) {
  if (!value) return '—';
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return '—';
  return new Intl.DateTimeFormat('zh-CN', {
    month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', hourCycle: 'h23'
  }).format(date).replaceAll('/', '-');
}

function fmtTime(value) {
  if (!value) return '—';
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return '—';
  return new Intl.DateTimeFormat('zh-CN', { hour: '2-digit', minute: '2-digit', second: '2-digit', hourCycle: 'h23' }).format(date);
}

function timeAgo(value) {
  if (!value) return '—';
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return '—';
  const seconds = Math.max(0, (Date.now() - date.getTime()) / 1000);
  if (seconds < 40) return '刚刚';
  if (seconds < 3600) return `${Math.floor(seconds / 60)} 分钟前`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)} 小时前`;
  if (seconds < 86400 * 3) return `${Math.floor(seconds / 86400)} 天前`;
  return fmtDate(value);
}

function fmtDuration(seconds) {
  if (!Number.isFinite(seconds) || seconds <= 0) return '—';
  const mins = Math.round(seconds / 60);
  if (mins < 60) return `${mins} 分钟`;
  return `${Math.floor(mins / 60)} 小时 ${mins % 60} 分`;
}

function statusOf(vehicle) {
  return STATUS[vehicle?.state?.status || vehicle?.status] || STATUS.offline;
}

function socOf(vehicle) {
  return Number(vehicle?.state?.soc ?? vehicle?.soc ?? 0);
}

function escapeAttr(value) {
  return escapeHtml(value).replaceAll('`', '&#096;');
}

function debounce(fn, wait = 300) {
  let timer;
  return (...args) => {
    clearTimeout(timer);
    timer = setTimeout(() => fn(...args), wait);
  };
}

function initials(name) {
  return String(name || '星').trim().slice(0, 1);
}

function roleLabel(role) {
  return { admin: '平台管理员', operator: '运营专员', viewer: '只读观察员' }[role] || '平台用户';
}

function uniqueId(prefix = 'id') {
  return `${prefix}-${Math.random().toString(36).slice(2, 9)}`;
}

function emptyState(title, text = '', iconName = 'car') {
  return `<div class="empty">${icon(iconName)}<strong>${escapeHtml(title)}</strong><span>${escapeHtml(text)}</span></div>`;
}

function toast(title, message = '', type = 'success') {
  const root = $('#toastRoot');
  const node = document.createElement('div');
  node.className = `toast ${type}`;
  node.innerHTML = `${icon(type === 'error' ? 'alert' : type === 'warning' ? 'info' : 'check')}
    <div><strong>${escapeHtml(title)}</strong>${message ? `<span>${escapeHtml(message)}</span>` : ''}</div>`;
  root.appendChild(node);
  setTimeout(() => {
    node.classList.add('out');
    setTimeout(() => node.remove(), 280);
  }, 4200);
}

async function api(path, options = {}) {
  const headers = { ...(options.headers || {}) };
  if (state.token) headers.Authorization = `Bearer ${state.token}`;
  if (options.body !== undefined) headers['Content-Type'] = 'application/json';
  let response;
  try {
    response = await fetch(`/api${path}`, {
      method: options.method || 'GET',
      headers,
      body: options.body !== undefined ? JSON.stringify(options.body) : undefined
    });
  } catch {
    throw new Error('无法连接服务器，请检查 TSP 服务是否运行');
  }
  if (response.status === 401) {
    if (state.token) logout(true);
    throw new Error('登录状态已失效，请重新登录');
  }
  if (response.status === 204) return null;
  let payload = null;
  try { payload = await response.json(); } catch { /* empty body */ }
  if (!response.ok) {
    throw new Error(payload?.error?.message || `请求失败（${response.status}）`);
  }
  return payload;
}

/* ------------------------------------------------------------------- auth */

function setLoginError(message) {
  const box = $('#loginError');
  if (!message) { box.classList.add('hidden'); box.textContent = ''; return; }
  box.textContent = message;
  box.classList.remove('hidden');
}

function showLogin() {
  $('#appView').classList.add('hidden');
  $('#loginView').classList.remove('hidden');
  $('#loginView').style.display = '';
  setTimeout(() => $('#username')?.focus(), 60);
}

function showApp() {
  $('#loginView').style.display = 'none';
  $('#appView').classList.remove('hidden');
}

async function enterApp() {
  const me = await api('/auth/me');
  state.user = me;
  localStorage.setItem('tsp_user', JSON.stringify(me));
  applyUser();
  showApp();
  connectWs();
  refreshClusterStatus();
  await navigate('dashboard');
}

async function refreshClusterStatus() {
  try {
    const cluster = await api('/cluster/status');
    const target = $('#clusterState');
    if (!target) return;
    if (!cluster.enabled) {
      target.textContent = '单机模式';
    } else if (cluster.raft_enabled) {
      target.textContent = cluster.leader ? 'Raft · Leader' : 'Raft · Follower';
    } else if (cluster.event_bus === 'iggy') {
      target.textContent = cluster.leader ? 'Iggy · Leader' : 'Iggy · Follower';
    } else if (cluster.leader) {
      target.textContent = `Redis · Leader`;
    } else {
      target.textContent = `Redis · Follower`;
    }
    target.title = `node: ${cluster.node_id}`;
  } catch { /* non-critical */ }
}

function applyUser() {
  if (!state.user) return;
  const short = initials(state.user.display_name);
  ['#topAvatar', '#sideAvatar', '#menuAvatar'].forEach((selector) => { const el = $(selector); if (el) el.textContent = short; });
  ['#topName', '#sideName', '#menuName'].forEach((selector) => { const el = $(selector); if (el) el.textContent = state.user.display_name; });
  ['#topRole', '#sideRole'].forEach((selector) => { const el = $(selector); if (el) el.textContent = roleLabel(state.user.role); });
  const username = $('#menuUsername'); if (username) username.textContent = state.user.username;
}

async function login(event) {
  event.preventDefault();
  const button = $('#loginBtn');
  const username = $('#username').value.trim();
  const password = $('#password').value;
  if (!username || !password) return;
  setLoginError('');
  button.disabled = true;
  button.innerHTML = `<span class="spinner"></span><span>正在验证</span>`;
  try {
    const data = await api('/auth/login', { method: 'POST', body: { username, password } });
    state.token = data.access_token || data.token;
    state.refreshToken = data.refresh_token || '';
    state.user = data.user;
    localStorage.setItem('tsp_token', state.token);
    localStorage.setItem('tsp_refresh_token', state.refreshToken);
    localStorage.setItem('tsp_user', JSON.stringify(data.user));
    scheduleAuthRefresh(data.expires_in);
    applyUser();
    showApp();
    connectWs();
    refreshClusterStatus();
    await navigate('dashboard');
    toast('登录成功', `欢迎回来，${data.user.display_name}`);
  } catch (error) {
    setLoginError(error.message);
    button.innerHTML = `<span>进入控制台</span>${icon('chevron')}`;
    button.disabled = false;
  }
}

function scheduleAuthRefresh(expiresIn) {
  clearTimeout(state.authRefreshTimer);
  const seconds = Number(expiresIn) || 900;
  const delay = Math.max(30, seconds - 60) * 1000;
  state.authRefreshTimer = setTimeout(refreshSession, delay);
}

async function refreshSession() {
  if (!state.refreshToken) { logout(true); return; }
  try {
    const data = await api('/auth/refresh', {
      method: 'POST',
      body: { refresh_token: state.refreshToken }
    });
    state.token = data.access_token;
    state.refreshToken = data.refresh_token;
    localStorage.setItem('tsp_token', state.token);
    localStorage.setItem('tsp_refresh_token', state.refreshToken);
    scheduleAuthRefresh(data.expires_in);
  } catch {
    logout(true);
  }
}

function logout(silent = false) {
  const token = state.token;
  const refreshToken = state.refreshToken;
  state.wsManualClose = true;
  state.ws?.close();
  clearTimeout(state.reconnectTimer);
  clearTimeout(state.authRefreshTimer);
  state.authRefreshTimer = null;
  if (token) {
    fetch('/api/auth/logout', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${token}` },
      body: JSON.stringify({ refresh_token: refreshToken || null }),
      keepalive: true
    }).catch(() => {});
  }
  state.token = '';
  state.refreshToken = '';
  state.user = null;
  localStorage.removeItem('tsp_token');
  localStorage.removeItem('tsp_refresh_token');
  localStorage.removeItem('tsp_user');
  state.map?.destroy();
  state.map = null;
  closeModal();
  if (!silent) toast('已退出登录', '账号凭证已清除');
  showLogin();
  $('#password').value = '';
  $('#loginBtn').innerHTML = `<span>进入控制台</span>${icon('chevron')}`;
  $('#loginBtn').disabled = false;
}

/* -------------------------------------------------------------- navigation */

async function navigate(view, options = {}) {
  if (!VIEW_META[view]) return;
  state.map?.destroy();
  state.map = null;
  state.view = view;
  $$('.nav-item').forEach((item) => item.classList.toggle('active', item.dataset.view === view));
  $('#pageTitle').textContent = VIEW_META[view][0];
  $('#crumbCurrent').textContent = VIEW_META[view][1];
  closeSidebar();
  $('#view').scrollTop = 0;
  $('#view').innerHTML = `<div class="loading"><span class="spinner"></span><span>正在加载 ${VIEW_META[view][0]} 数据</span></div>`;
  try {
    if (view === 'dashboard') await loadDashboard();
    else if (view === 'live') await loadLive(options.vehicleId);
    else if (view === 'vehicles') await loadVehicles();
    else if (view === 'trips') await loadTrips();
    else if (view === 'geofences') await loadGeofences();
    else if (view === 'alerts') await loadAlerts();
    else if (view === 'commands') await loadCommands();
    else if (view === 'terminals') await loadTerminals();
    else if (view === 'media') await loadMedia();
  } catch (error) {
    if ($('#appView').classList.contains('hidden')) return;
    renderViewError(error);
  }
}

function refreshCurrent() {
  if (state.view === 'live') navigate('live', { vehicleId: state.selectedVehicleId });
  else navigate(state.view);
}

function renderViewError(error) {
  $('#view').innerHTML = `
    <div class="page-head">
      <div class="page-head-copy"><h2>${VIEW_META[state.view][0]}</h2><p>数据加载未完成</p></div>
      <div class="page-head-actions"><button class="btn" data-action="refresh">${icon('refresh')}重新加载</button></div>
    </div>
    <div class="panel">${emptyState('暂时无法获取数据', error.message, 'alert')}</div>`;
}

async function loadMedia() {
  $('#view').innerHTML = `
    <div class="page-head">
      <div class="page-head-copy"><h2>媒体播放</h2><p>JT/T 1078 实时会话与 fMP4/MSE 预览</p></div>
      <div class="page-head-actions"><a class="btn" href="/media" target="_blank" rel="noopener">新窗口打开</a></div>
    </div>
    <div class="panel" style="padding:0;overflow:hidden;height:calc(100vh - 190px);min-height:420px">
      <iframe src="/media" title="JT/T 1078 媒体播放" style="width:100%;height:100%;border:0;background:#060b14"></iframe>
    </div>`;
}

/* --------------------------------------------------------------- websocket */

function setWsState(connected) {
  state.wsReady = connected;
  const live = $('#liveStatus');
  const ws = $('#wsState');
  if (live) live.classList.toggle('offline', !connected);
  if (live) live.querySelector('span').textContent = connected ? '实时连接' : '连接中断';
  if (ws) ws.textContent = connected ? '已连接' : '重连中';
  $('#connectionBanner')?.classList.toggle('hidden', connected);
}

function connectWs() {
  if (!state.token) return;
  state.wsManualClose = false;
  clearTimeout(state.reconnectTimer);
  try { state.ws?.close(); } catch { /* ignore */ }
  const protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
  const socket = new WebSocket(`${protocol}//${location.host}/api/ws`, [`bearer.${state.token}`]);
  state.ws = socket;
  socket.addEventListener('open', () => setWsState(true));
  socket.addEventListener('message', (event) => {
    try {
      const payload = JSON.parse(event.data);
      handleRealtime(payload);
    } catch { /* malformed frame */ }
  });
  socket.addEventListener('close', () => {
    if (state.ws !== socket) return;
    setWsState(false);
    if (!state.wsManualClose && state.token) {
      clearTimeout(state.reconnectTimer);
      state.reconnectTimer = setTimeout(connectWs, 3500);
    }
  });
  socket.addEventListener('error', () => setWsState(false));
}

function handleRealtime(event) {
  if (!event || !event.type) return;
  if (event.type === 'ready') { setWsState(true); return; }
  if (event.type === 'resync') { if (state.view !== 'dashboard') refreshCurrent(); return; }

  if (event.type === 'telemetry') {
    const patch = {
      lat: event.lat, lon: event.lon, speed: event.speed, heading: event.heading,
      ignition: event.ignition, soc: event.soc, status: event.status, odometer_km: event.odometer_km,
      updated_at: event.ts
    };
    const vehicle = state.vehicles.find((item) => item.id === event.vehicle_id);
    if (vehicle) { Object.assign(vehicle, patch); if (vehicle.state) Object.assign(vehicle.state, patch); }
    const full = state.allVehicles.find((item) => item.id === event.vehicle_id);
    if (full?.state) Object.assign(full.state, patch);
    if (state.view === 'live' && state.selectedVehicleId === event.vehicle_id) {
      const last = state.liveTrajectory[state.liveTrajectory.length - 1];
      if (!last || new Date(event.ts) > new Date(last.ts)) {
        state.liveTrajectory.push({ vehicle_id: event.vehicle_id, ts: event.ts, lat: event.lat, lon: event.lon, speed: event.speed, heading: event.heading });
        if (state.liveTrajectory.length > 600) state.liveTrajectory.shift();
      }
      state.selectedVehicle = state.selectedVehicle && { ...state.selectedVehicle, state: { ...state.selectedVehicle.state, ...patch } };
      updateLiveTelemetry(event);
    }
    if (state.view === 'dashboard' || state.view === 'live') {
      state.map?.setData({
        vehicles: state.vehicles,
        geofences: state.geofences,
        trajectory: state.view === 'live' ? state.liveTrajectory : [],
        selectedId: state.selectedVehicleId
      });
    }
    if (state.view === 'live') updateLiveRailRow(event);
    return;
  }

  if (event.type === 'alert') {
    const alert = event.alert;
    state.alertUnread += 1;
    updateAlertBadge();
    const level = LEVEL[alert.level] || LEVEL.info;
    toast(`${level.label}告警 · ${alert.title}`, alert.message, alert.level === 'critical' ? 'error' : 'warning');
    if (state.view === 'alerts') {
      clearTimeout(state.refreshTimer);
      state.refreshTimer = setTimeout(() => loadAlerts(), 900);
    } else if (state.view === 'dashboard') {
      clearTimeout(state.refreshTimer);
      state.refreshTimer = setTimeout(() => loadDashboard(), 5000);
    } else if (state.view === 'live' && alert.vehicle_id === state.selectedVehicleId) {
      state.liveAlerts.unshift(alert);
      state.liveAlerts = state.liveAlerts.slice(0, 4);
      renderLiveAlerts();
    }
    return;
  }

  if (event.type === 'command') {
    const command = event.command;
    if (command.status === 'success') toast('指令执行成功', `${command.plate_no || ''} · ${COMMAND_LABEL[command.command]?.label || command.command}`);
    if (command.status === 'failed') toast('指令执行失败', command.result || 'T-Box 响应异常', 'error');
    if (state.view === 'commands') {
      clearTimeout(state.refreshTimer);
      state.refreshTimer = setTimeout(() => loadCommands(), 700);
    }
    return;
  }

  if (event.type === 'fleet') {
    const badge = $('#navVehicleCount');
    if (badge) badge.textContent = state.dashboard?.fleet?.total ?? badge.textContent;
  }
}

function updateAlertBadge() {
  const badge = $('#navAlertCount');
  if (!badge) return;
  const count = state.alertUnread || 0;
  badge.textContent = count > 99 ? '99+' : String(count);
  badge.classList.toggle('hidden', count <= 0);
}

/* ---------------------------------------------------------------- dashboard */

async function loadDashboard() {
  const [dashboard, vehicles, geofences] = await Promise.all([
    api('/dashboard'),
    api('/vehicles/map'),
    api('/geofences')
  ]);
  state.dashboard = dashboard;
  state.vehicles = vehicles;
  state.geofences = geofences;
  state.alertUnread = dashboard.alerts.active;
  updateAlertBadge();
  const count = $('#navVehicleCount');
  if (count) count.textContent = dashboard.fleet.total;
  renderDashboard();
}

function sparkline(values, color = '#2dd4bf') {
  const data = values.map(Number).filter(Number.isFinite);
  if (data.length < 2) return '';
  const width = 90, height = 26, max = Math.max(...data, 1), min = Math.min(...data, 0);
  const points = data.map((value, index) => {
    const x = (index / (data.length - 1)) * width;
    const y = height - 3 - ((value - min) / Math.max(1, max - min)) * (height - 6);
    return `${x.toFixed(1)},${y.toFixed(1)}`;
  }).join(' ');
  return `<svg class="kpi-spark" viewBox="0 0 ${width} ${height}" preserveAspectRatio="none">
    <polyline points="${points}" fill="none" stroke="${color}" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" opacity=".85"/>
  </svg>`;
}

function lineChart(data, color = '#2dd4bf', unit = '') {
  if (!data || data.length === 0) return emptyState('暂无趋势数据', '', 'route');
  const width = 720, height = 205;
  const pad = { left: 40, right: 16, top: 18, bottom: 28 };
  const values = data.map((item) => Number(item.value) || 0);
  const maxValue = Math.max(...values, 1);
  const magnitude = Math.pow(10, Math.max(0, Math.floor(Math.log10(maxValue)) - 1));
  const max = Math.ceil(maxValue / magnitude) * magnitude * 1.08 || 1;
  const plotW = width - pad.left - pad.right;
  const plotH = height - pad.top - pad.bottom;
  const xs = data.map((_, index) => pad.left + (index / Math.max(1, data.length - 1)) * plotW);
  const ys = values.map((value) => pad.top + plotH - (value / max) * plotH);
  let path = `M ${xs[0]} ${ys[0]}`;
  for (let index = 1; index < xs.length; index += 1) {
    const mid = (xs[index - 1] + xs[index]) / 2;
    path += ` C ${mid} ${ys[index - 1]}, ${mid} ${ys[index]}, ${xs[index]} ${ys[index]}`;
  }
  const area = `${path} L ${xs[xs.length - 1]} ${pad.top + plotH} L ${xs[0]} ${pad.top + plotH} Z`;
  const gradientId = uniqueId('chart');
  const grid = [0, .25, .5, .75, 1].map((ratio) => {
    const y = pad.top + plotH - ratio * plotH;
    const label = max * ratio;
    return `<line x1="${pad.left}" y1="${y}" x2="${width - pad.right}" y2="${y}" stroke="rgba(147,179,209,.08)" stroke-dasharray="3 5"/>
      <text x="${pad.left - 9}" y="${y + 3}" text-anchor="end" fill="#5b7187" font-size="8.5" font-family="monospace">${label >= 1000 ? (label / 1000).toFixed(1) + 'k' : Math.round(label)}</text>`;
  }).join('');
  const labels = data.map((item, index) => `<text x="${xs[index]}" y="${height - 8}" text-anchor="middle" fill="#5b7187" font-size="8.5">${escapeHtml(item.label)}</text>`).join('');
  const dots = data.map((item, index) => `<circle cx="${xs[index]}" cy="${ys[index]}" r="${index === data.length - 1 ? 3.4 : 2}" fill="${index === data.length - 1 ? color : '#0c1928'}" stroke="${color}" stroke-width="1.2"/>`).join('');
  return `<svg class="chart-svg" viewBox="0 0 ${width} ${height}" preserveAspectRatio="none" aria-hidden="true">
    <defs><linearGradient id="${gradientId}" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="${color}" stop-opacity=".26"/><stop offset="1" stop-color="${color}" stop-opacity="0"/>
    </linearGradient></defs>
    ${grid}
    <path d="${area}" fill="url(#${gradientId})"/>
    <path d="${path}" fill="none" stroke="${color}" stroke-width="1.9" stroke-linecap="round" vector-effect="non-scaling-stroke"/>
    ${dots}${labels}
    <text x="${pad.left}" y="10" fill="#5b7187" font-size="8">${unit}</text>
  </svg>`;
}

function donutData(distribution) {
  const total = distribution.reduce((sum, item) => sum + Number(item.value || 0), 0) || 1;
  let offset = 0;
  const segments = distribution.filter((item) => item.value > 0).map((item) => {
    const start = offset;
    offset += (Number(item.value) / total) * 100;
    return `${STATUS[item.name]?.color || '#5b7187'} ${start.toFixed(2)}% ${offset.toFixed(2)}%`;
  });
  return `conic-gradient(${segments.join(', ') || '#243a4e 0 100%'})`;
}

function renderDashboard() {
  const dashboard = state.dashboard;
  if (!dashboard) return;
  const fleet = dashboard.fleet;
  const alerts = dashboard.alerts;
  const todayMileage = dashboard.mileage_7d[dashboard.mileage_7d.length - 1]?.value || 0;
  const total7d = dashboard.mileage_7d.reduce((sum, item) => sum + Number(item.value || 0), 0);
  const distribution = dashboard.status_distribution;
  const activeStatuses = distribution.filter((item) => item.value > 0);
  const statusRows = activeStatuses.map((item) => {
    const meta = STATUS[item.name] || { label: item.name, color: '#5b7187' };
    const percent = fleet.total ? (item.value / fleet.total) * 100 : 0;
    return `<div class="health-row"><span>${meta.label}</span><span class="bar"><i style="width:${percent.toFixed(1)}%;background:${meta.color}"></i></span><b>${item.value}</b></div>`;
  }).join('');
  const alertMax = Math.max(...dashboard.alert_trend_7d.map((item) => Number(item.value)), 1);
  const alertBars = dashboard.alert_trend_7d.map((item) => `<div class="mini-bar"><i style="height:${Math.max(10, Math.pow(Number(item.value) / alertMax, 0.55) * 100)}%"></i><span>${escapeHtml(String(item.label).slice(-2))}</span></div>`).join('');

  const recentAlerts = dashboard.recent_alerts.slice(0, 4).map((alert) => {
    const level = LEVEL[alert.level] || LEVEL.info;
    const meta = ALERT_META[alert.alert_type] || { label: '告警', icon: 'bell' };
    return `<div class="alert-item" data-action="alert-focus" data-id="${escapeAttr(alert.id)}">
      <span class="alert-ico ${alert.level}">${icon(meta.icon)}</span>
      <div class="alert-main">
        <div class="alert-title-row"><strong>${escapeHtml(alert.title)}</strong><time>${timeAgo(alert.ts)}</time></div>
        <div class="alert-desc">${escapeHtml(alert.message)}</div>
        <div class="alert-meta"><b>${escapeHtml(alert.plate_no || '—')}</b><span>·</span><span>${escapeHtml(meta.label)}</span><span class="badge ${level.badge}" style="margin-left:auto">${level.label}</span></div>
      </div>
    </div>`;
  }).join('');

  $('#view').innerHTML = `
    <div class="page-head">
      <div class="page-head-copy">
        <h2>车队运营总览</h2>
        <p>实时感知车辆与终端状态，聚合今日运营、风险与指令执行情况</p>
      </div>
      <div class="page-head-actions">
        <span class="badge badge-muted">${icon('clock')} 数据更新于 ${fmtTime(dashboard.generated_at)}</span>
        <button class="btn" data-action="refresh">${icon('refresh')}刷新数据</button>
      </div>
    </div>

    <div class="kpi-grid">
      <div class="kpi-card" style="--accent:rgba(45,212,191,.18);--accent-solid:#2dd4bf">
        <div class="kpi-top"><span class="kpi-label">${icon('car')}在线车辆</span><span class="kpi-delta">在线率 ${fleet.online_rate}%</span></div>
        <div class="kpi-value"><strong>${fleet.online}</strong><span>/ ${fleet.total} 台</span></div>
        <div class="kpi-foot"><span>行驶 ${fleet.moving} · 怠速 ${fleet.idle} · 充电 ${fleet.charging}</span>${sparkline([fleet.online - 2, fleet.online - 1, fleet.online, fleet.online - 1, fleet.online, fleet.online + 1, fleet.online], '#2dd4bf')}</div>
      </div>
      <div class="kpi-card" style="--accent:rgba(79,140,255,.18);--accent-solid:#4f8cff">
        <div class="kpi-top"><span class="kpi-label">${icon('route')}今日里程</span><span class="kpi-delta">7日 ${fmtNumber(total7d)} km</span></div>
        <div class="kpi-value"><strong>${fmtKm(todayMileage)}</strong><span>km</span></div>
        <div class="kpi-foot"><span>累计运营里程 ${fmtNumber(fleet.total_mileage_km)} km</span>${sparkline(dashboard.mileage_7d.map((item) => item.value), '#4f8cff')}</div>
      </div>
      <div class="kpi-card" style="--accent:rgba(255,95,116,.16);--accent-solid:#ff5f74">
        <div class="kpi-top"><span class="kpi-label">${icon('bell')}活跃告警</span><span class="kpi-delta" style="color:#ff8394;background:rgba(255,95,116,.09)">严重 ${alerts.critical}</span></div>
        <div class="kpi-value"><strong>${alerts.active}</strong><span>条待处置</span></div>
        <div class="kpi-foot"><span>今日新增 ${alerts.today} 条 · 警告 ${alerts.warning}</span>${sparkline(dashboard.alert_trend_7d.map((item) => item.value), '#ff5f74')}</div>
      </div>
      <div class="kpi-card" style="--accent:rgba(246,179,75,.16);--accent-solid:#f6b34b">
        <div class="kpi-top"><span class="kpi-label">${icon('command')}指令成功率</span><span class="kpi-delta">近7日</span></div>
        <div class="kpi-value"><strong>${fleet.total ? dashboard.command.success_rate.toFixed(1) : '—'}</strong><span>%</span></div>
        <div class="kpi-foot"><span>共下发 ${dashboard.command.total_7d} 条指令</span>${sparkline([86, 90, 88, 93, 91, 95, dashboard.command.success_rate || 0], '#f6b34b')}</div>
      </div>
    </div>

    <div class="dashboard-grid">
      <section class="panel map-panel span-8">
        <div class="panel-head">
          <div class="panel-title">${icon('radar')}<div><h3>车队实时分布</h3><p>上海区域 · ${fleet.online} 台在线车辆</p></div></div>
          <div class="panel-tools"><span class="badge badge-live"><i></i>LIVE</span><button class="panel-link" data-nav="live">进入实时监控 ${icon('chevron')}</button></div>
        </div>
        <div class="map-wrap">
          <canvas id="dashMap" class="map-canvas" aria-label="车队实时地图"></canvas>
          <div class="map-overlay-top">
            <span class="map-chip"><span class="dot"></span>在线 <b>${fleet.online}</b></span>
            <span class="map-chip">告警 <b>${alerts.active}</b></span>
            <span class="map-chip">围栏 <b>${state.geofences.length}</b></span>
          </div>
        </div>
        <div class="map-legend">
          <span><i style="background:#2dd4bf"></i>行驶中</span>
          <span><i style="background:#6fa6ff"></i>怠速</span>
          <span><i style="background:#f6b34b"></i>充电</span>
          <span><i style="background:#667d92"></i>离线</span>
          <span class="map-hint">示意地图 · 实时坐标由 T-Box 上报</span>
        </div>
      </section>

      <section class="panel span-4">
        <div class="panel-head">
          <div class="panel-title">${icon('gauge')}<div><h3>车队健康度</h3><p>车辆状态与终端在线情况</p></div></div>
          <span class="badge badge-muted">${fleet.total} 台</span>
        </div>
        <div class="health-body">
          <div class="health-main">
            <div class="donut" style="background:${donutData(distribution)}">
              <div class="donut-center"><strong>${fleet.online_rate}%</strong><span>车辆在线率</span></div>
            </div>
            <div class="health-legend">${statusRows}</div>
          </div>
          <div class="health-meta">
            <div><span>平均 SOC</span><strong>${fleet.avg_soc}%</strong></div>
            <div><span>终端在线率</span><strong>${fleet.terminal_online_rate}%</strong></div>
          </div>
          <div class="health-extra">
            <div class="health-extra-head"><span>近 7 日告警趋势</span><b>今日 ${alerts.today} 条</b></div>
            <div class="mini-bars">${alertBars}</div>
          </div>
          <div class="health-note">${icon('info')}<span>${fleet.offline} 台车辆离线，${alerts.critical} 条严重告警待优先处置；建议关注低电量与碰撞事件。</span></div>
        </div>
      </section>

      <section class="panel span-7">
        <div class="panel-head">
          <div class="panel-title">${icon('route')}<div><h3>近 7 日运营里程</h3><p>按自然日汇总车辆行驶里程</p></div></div>
          <span class="badge badge-muted">单位 km</span>
        </div>
        <div class="chart-box">${lineChart(dashboard.mileage_7d, '#2dd4bf', 'km')}</div>
      </section>

      <section class="panel span-5">
        <div class="panel-head">
          <div class="panel-title">${icon('bell')}<div><h3>最新风险事件</h3><p>按发生时间倒序 · 待处置 ${alerts.active} 条</p></div></div>
          <button class="panel-link" data-nav="alerts">全部告警 ${icon('chevron')}</button>
        </div>
        <div class="alert-feed">${recentAlerts || emptyState('暂无风险事件', '车队运行平稳', 'shield')}</div>
      </section>
    </div>`;

  state.map = new FleetMap($('#dashMap'), {
    vehicles: state.vehicles,
    geofences: state.geofences,
    onSelect: (id) => { state.selectedVehicleId = id; navigate('live', { vehicleId: id }); }
  });
  bindDashboard();
}

function bindDashboard() {
  // delegated actions handle the rest
}

/* ------------------------------------------------------------------- live */

async function loadLive(vehicleId) {
  const [vehicles, geofences, all] = await Promise.all([
    api('/vehicles/map'),
    api('/geofences'),
    api('/vehicles?page_size=200')
  ]);
  state.vehicles = vehicles;
  state.geofences = geofences;
  state.allVehicles = all.items || [];
  state.liveTrajectory = [];
  state.liveAlerts = [];
  const requested = vehicleId || state.selectedVehicleId;
  const preferred = state.allVehicles.find((item) => item.id === requested)
    || state.allVehicles.find((item) => item.state?.status === 'moving')
    || state.allVehicles[0];
  state.selectedVehicleId = preferred?.id || null;
  renderLive();
  if (state.selectedVehicleId) await selectLiveVehicle(state.selectedVehicleId, true);
}

function renderLive() {
  const vehicles = state.allVehicles;
  const listHtml = renderLiveVehicleRows();
  $('#view').innerHTML = `
    <div class="page-head" style="margin-bottom:14px">
      <div class="page-head-copy">
        <h2>实时监控</h2>
        <p>车辆位置、状态与轨迹秒级刷新，支持远程指令即时下发</p>
      </div>
      <div class="page-head-actions">
        <span class="live-status"><i></i><span>实时连接</span></span>
        <button class="btn" data-action="refresh">${icon('refresh')}刷新</button>
      </div>
    </div>
    <div class="live-layout">
      <section class="panel vehicle-rail">
        <div class="panel-head"><div class="panel-title">${icon('car')}<div><h3>车队列表</h3><p>${vehicles.length} 台车辆</p></div></div><span class="badge badge-live"><i></i>${vehicles.filter((item) => item.state?.status !== 'offline').length}</span></div>
        <div class="rail-search">${icon('search')}<input id="liveSearch" type="search" placeholder="搜索车牌 / 驾驶员" value="${escapeAttr(state.liveSearch)}"></div>
        <div class="rail-filters"><div class="segmented" id="liveFilters">
          <button data-filter="all" class="${state.liveFilter === 'all' ? 'active' : ''}">全部</button>
          <button data-filter="moving" class="${state.liveFilter === 'moving' ? 'active' : ''}">行驶</button>
          <button data-filter="offline" class="${state.liveFilter === 'offline' ? 'active' : ''}">离线</button>
        </div></div>
        <div class="vehicle-list" id="liveVehicleList">${listHtml}</div>
      </section>

      <section class="panel map-panel live-map-panel">
        <div class="live-map-head">
          <div class="live-map-head-left"><strong id="livePlate">—</strong><span id="liveModel">—</span><span class="status-pill" id="liveStatusPill">—</span></div>
          <div class="panel-tools">
            <button class="btn btn-sm" data-action="dispatch-command" data-id="${escapeAttr(state.selectedVehicleId || '')}">${icon('command')}远程指令</button>
          </div>
        </div>
        <div class="map-wrap">
          <canvas id="liveMap" class="map-canvas" aria-label="车辆实时位置"></canvas>
          <div class="map-overlay-top" id="liveMapOverlay"></div>
        </div>
        <div class="live-telemetry">
          <div><span>当前车速</span><strong><b id="telemetrySpeed">—</b><small>km/h</small></strong></div>
          <div><span>剩余电量</span><strong><b id="telemetrySoc">—</b><small>%</small></strong></div>
          <div><span>累计里程</span><strong><b id="telemetryOdo">—</b><small>km</small></strong></div>
          <div><span>最后上报</span><strong style="font-size:11px"><b id="telemetryUpdated">—</b></strong></div>
        </div>
      </section>

      <section class="panel detail-rail" id="liveDetail">${liveDetailSkeleton()}</section>
    </div>`;

  state.map = new FleetMap($('#liveMap'), {
    vehicles: state.vehicles,
    geofences: state.geofences,
    trajectory: state.liveTrajectory,
    selectedId: state.selectedVehicleId,
    onSelect: (id) => selectLiveVehicle(id)
  });
  bindLive();
  setWsState(state.wsReady);
  updateLiveHeader();
}

function liveDetailSkeleton() {
  return `<div class="detail-section" style="padding:18px">
    <div class="skeleton" style="width:45%;height:12px;margin-bottom:12px"></div>
    <div class="skeleton" style="width:70%;height:24px;margin-bottom:8px"></div>
    <div class="skeleton" style="width:56%;height:10px;margin-bottom:22px"></div>
    <div class="skeleton" style="width:100%;height:66px;margin-bottom:12px"></div>
    <div class="skeleton" style="width:100%;height:112px"></div>
  </div>`;
}

function liveVehicleFiltered() {
  const query = state.liveSearch.trim().toLowerCase();
  return state.allVehicles.filter((vehicle) => {
    const status = vehicle.state?.status || 'offline';
    if (state.liveFilter === 'moving' && status !== 'moving') return false;
    if (state.liveFilter === 'offline' && status !== 'offline') return false;
    if (!query) return true;
    return `${vehicle.plate_no} ${vehicle.model} ${vehicle.driver_name || ''}`.toLowerCase().includes(query);
  });
}

function renderLiveVehicleRows() {
  const vehicles = liveVehicleFiltered();
  if (!vehicles.length) return emptyState('没有匹配车辆', '调整筛选条件后重试', 'search');
  return vehicles.map((vehicle) => {
    const meta = statusOf(vehicle);
    const soc = socOf(vehicle);
    const active = vehicle.id === state.selectedVehicleId;
    return `<button class="vehicle-row ${active ? 'active' : ''}" data-action="select-live" data-id="${escapeAttr(vehicle.id)}">
      <span class="vehicle-status-ico ${meta.className.replace('status-', '')}">${icon(meta.icon)}</span>
      <span class="vehicle-row-main">
        <span class="vehicle-row-top"><strong>${escapeHtml(vehicle.plate_no)}</strong><span>${escapeHtml(meta.label)}</span></span>
        <span class="vehicle-row-sub">${escapeHtml(vehicle.model)} · ${escapeHtml(vehicle.driver_name || '未指派')}</span>
        <span class="vehicle-row-bot"><span class="soc-mini"><i style="width:${Math.max(3, soc)}%;background:${soc < 20 ? '#ff5f74' : meta.color}"></i></span><span>${Math.round(soc)}%</span><span style="margin-left:auto">${vehicle.state?.speed ? `${Math.round(vehicle.state.speed)} km/h` : '静止'}</span></span>
      </span>
    </button>`;
  }).join('');
}

function bindLive() {
  const search = $('#liveSearch');
  if (search) search.addEventListener('input', debounce(() => {
    state.liveSearch = search.value;
    $('#liveVehicleList').innerHTML = renderLiveVehicleRows();
  }, 220));
  $$('#liveFilters button').forEach((button) => button.addEventListener('click', () => {
    state.liveFilter = button.dataset.filter;
    $$('#liveFilters button').forEach((item) => item.classList.toggle('active', item === button));
    $('#liveVehicleList').innerHTML = renderLiveVehicleRows();
  }));
}

function updateLiveHeader() {
  const vehicle = state.allVehicles.find((item) => item.id === state.selectedVehicleId);
  if (!vehicle) return;
  const meta = statusOf(vehicle);
  $('#livePlate').textContent = vehicle.plate_no || '—';
  $('#liveModel').textContent = vehicle.model || '—';
  const pill = $('#liveStatusPill');
  pill.className = `status-pill ${meta.className}`;
  pill.innerHTML = `<i></i>${meta.label}`;
  $('#telemetrySpeed').textContent = Math.round(vehicle.state?.speed || 0);
  $('#telemetrySoc').textContent = Math.round(vehicle.state?.soc || 0);
  $('#telemetryOdo').textContent = fmtNumber(vehicle.state?.odometer_km || 0);
  $('#telemetryUpdated').textContent = timeAgo(vehicle.state?.updated_at);
  $('#liveMapOverlay').innerHTML = `
    <span class="map-chip"><span class="dot"></span>${escapeHtml(meta.label)}</span>
    <span class="map-chip">SOC <b>${Math.round(socOf(vehicle))}%</b></span>
    <span class="map-chip">轨迹点 <b id="trajectoryCount">${state.liveTrajectory.length}</b></span>`;
  const commandButton = $('.live-map-head [data-action="dispatch-command"]');
  if (commandButton) commandButton.dataset.id = vehicle.id;
}

function updateLiveTelemetry(event) {
  const speed = $('#telemetrySpeed');
  if (speed) speed.textContent = Math.round(event.speed || 0);
  const soc = $('#telemetrySoc');
  if (soc) soc.textContent = Math.round(event.soc || 0);
  const odo = $('#telemetryOdo');
  if (odo) odo.textContent = fmtNumber(event.odometer_km || 0);
  const updated = $('#telemetryUpdated');
  if (updated) updated.textContent = '刚刚';
  const count = $('#trajectoryCount');
  if (count) count.textContent = state.liveTrajectory.length;
  const battery = $('#detailBatteryFill');
  if (battery) {
    battery.style.width = `${Math.max(2, Math.min(100, Number(event.soc) || 0))}%`;
    battery.classList.toggle('low', Number(event.soc) < 20);
  }
  const batteryLabel = $('#detailBatteryValue');
  if (batteryLabel) batteryLabel.textContent = `${Math.round(event.soc || 0)}%`;
  const speedText = $('#detailSpeed');
  if (speedText) speedText.textContent = `${Math.round(event.speed || 0)} km/h`;
}

function updateLiveRailRow(event) {
  const row = $(`[data-action="select-live"][data-id="${CSS.escape(event.vehicle_id)}"]`);
  if (!row) return;
  const speed = row.querySelector('.vehicle-row-bot span:last-child');
  if (speed) speed.textContent = event.speed > 1 ? `${Math.round(event.speed)} km/h` : '静止';
  const soc = row.querySelector('.vehicle-row-bot span:nth-child(2)');
  if (soc) soc.textContent = `${Math.round(event.soc || 0)}%`;
  const mini = row.querySelector('.soc-mini i');
  if (mini) mini.style.width = `${Math.max(3, Number(event.soc) || 0)}%`;
}

async function selectLiveVehicle(id, initial = false) {
  state.selectedVehicleId = id;
  $$('[data-action="select-live"]').forEach((row) => row.classList.toggle('active', row.dataset.id === id));
  const vehicle = state.allVehicles.find((item) => item.id === id);
  if (!vehicle) return;
  state.selectedVehicle = vehicle;
  updateLiveHeader();
  try {
    const [trajectory, alertPage] = await Promise.all([
      api(`/vehicles/${encodeURIComponent(id)}/trajectory?limit=500`),
      api(`/alerts?vehicle_id=${encodeURIComponent(id)}&page_size=4`)
    ]);
    state.liveTrajectory = trajectory || [];
    state.liveAlerts = alertPage.items || [];
    state.map?.setData({
      vehicles: state.vehicles,
      geofences: state.geofences,
      trajectory: state.liveTrajectory,
      selectedId: id
    });
    renderLiveDetail(vehicle);
    updateLiveHeader();
    if (!initial) {
      const row = $(`[data-action="select-live"][data-id="${CSS.escape(id)}"]`);
      row?.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
    }
  } catch (error) {
    toast('轨迹加载失败', error.message, 'error');
  }
}

function renderLiveDetail(vehicle) {
  const stateInfo = vehicle.state || {};
  const meta = statusOf(vehicle);
  const soc = Number(stateInfo.soc || 0);
  const alerts = state.liveAlerts.map((alert) => {
    const level = LEVEL[alert.level] || LEVEL.info;
    return `<div class="mini-alert ${alert.level}"><span class="dot"></span><div><strong>${escapeHtml(alert.title)}</strong><span>${timeAgo(alert.ts)} · ${level.label}</span></div></div>`;
  }).join('');
  $('#liveDetail').innerHTML = `
    <div class="panel-head"><div class="panel-title">${icon('car')}<div><h3>车辆详情</h3><p>${escapeHtml(vehicle.org_name || '星途车队')}</p></div></div><span class="status-pill ${meta.className}"><i></i>${meta.label}</span></div>
    <div class="vehicle-hero">
      <div class="vehicle-hero-top"><span class="tag">${escapeHtml(vehicle.energy_type || 'EV')}</span><span class="text-muted" style="font-size:9px">${escapeHtml(vehicle.color || '')}</span></div>
      <h3>${escapeHtml(vehicle.plate_no)}</h3>
      <p>${escapeHtml(vehicle.model)} · ${escapeHtml(vehicle.vin)}</p>
      <div class="hero-battery">
        <div class="hero-battery-top"><span>动力电池 SOC</span><b id="detailBatteryValue">${Math.round(soc)}%</b></div>
        <div class="battery-bar"><i id="detailBatteryFill" class="${soc < 20 ? 'low' : ''}" style="width:${Math.max(2, Math.min(100, soc))}%"></i></div>
      </div>
    </div>
    <div class="detail-section">
      <div class="detail-section-title">实时状态</div>
      <div class="detail-grid">
        <div class="detail-item"><span>当前车速</span><b id="detailSpeed">${Math.round(stateInfo.speed || 0)} km/h</b></div>
        <div class="detail-item"><span>累计里程</span><b>${fmtKm(stateInfo.odometer_km || 0)} km</b></div>
        <div class="detail-item"><span>点火状态</span><b>${stateInfo.ignition ? '已启动' : '已熄火'}</b></div>
        <div class="detail-item"><span>最后上报</span><b>${timeAgo(stateInfo.updated_at)}</b></div>
        <div class="detail-item"><span>驾驶员</span><b>${escapeHtml(vehicle.driver_name || '未指派')}</b></div>
        <div class="detail-item"><span>联系电话</span><b>${escapeHtml(vehicle.driver_phone || '—')}</b></div>
        <div class="detail-item"><span>终端编号</span><b class="mono">${escapeHtml(vehicle.terminal?.phone || vehicle.terminal?.imei || '未绑定')}</b></div>
        <div class="detail-item"><span>坐标</span><b class="mono">${stateInfo.lat ? `${Number(stateInfo.lat).toFixed(5)}, ${Number(stateInfo.lon).toFixed(5)}` : '—'}</b></div>
      </div>
    </div>
    <div class="detail-section">
      <div class="detail-section-title">快捷控制</div>
      <div class="quick-actions">
        ${canWrite() ? `
        <button class="quick-action" data-action="quick-command" data-id="${escapeAttr(vehicle.id)}" data-command="lock">${icon('lock')}上锁</button>
        <button class="quick-action" data-action="quick-command" data-id="${escapeAttr(vehicle.id)}" data-command="unlock">${icon('unlock')}解锁</button>
        <button class="quick-action" data-action="quick-command" data-id="${escapeAttr(vehicle.id)}" data-command="ac_on">${icon('snow')}开空调</button>
        <button class="quick-action" data-action="quick-command" data-id="${escapeAttr(vehicle.id)}" data-command="find">${icon('pin')}寻车</button>
        <button class="quick-action" data-action="quick-command" data-id="${escapeAttr(vehicle.id)}" data-command="flash">${icon('locate')}闪灯</button>
        <button class="quick-action" data-action="dispatch-command" data-id="${escapeAttr(vehicle.id)}">${icon('more')}更多</button>` : `<div class="text-muted" style="grid-column:span 3;font-size:9px;padding:5px 0">当前账号为只读权限</div>`}
      </div>
    </div>
    <div class="detail-section">
      <div class="detail-section-title">最近告警</div>
      ${alerts || '<div class="text-muted" style="font-size:9px">暂无告警记录</div>'}
    </div>`;
}

function renderLiveAlerts() {
  const section = $$('#liveDetail .detail-section').at(-1);
  if (!section) return;
  const content = state.liveAlerts.map((alert) => {
    const level = LEVEL[alert.level] || LEVEL.info;
    return `<div class="mini-alert ${alert.level}"><span class="dot"></span><div><strong>${escapeHtml(alert.title)}</strong><span>${timeAgo(alert.ts)} · ${level.label}</span></div></div>`;
  }).join('');
  section.innerHTML = `<div class="detail-section-title">最近告警</div>${content || '<div class="text-muted" style="font-size:9px">暂无告警记录</div>'}`;
}

/* ---------------------------------------------------------------- vehicles */

async function loadVehicles() {
  const query = new URLSearchParams({
    page: String(state.vehiclePage),
    page_size: String(state.vehiclePageSize)
  });
  if (state.vehicleSearch) query.set('search', state.vehicleSearch);
  if (state.vehicleStatus) query.set('status', state.vehicleStatus);
  const page = await api(`/vehicles?${query}`);
  renderVehicles(page);
}

function renderVehicles(page) {
  const rows = page.items.map((vehicle) => {
    const meta = statusOf(vehicle);
    const soc = socOf(vehicle);
    const terminal = vehicle.terminal;
    return `<tr class="clickable" data-action="vehicle-detail" data-id="${escapeAttr(vehicle.id)}">
      <td><div class="cell-vehicle"><span class="vehicle-avatar">${icon('car')}</span><div><strong>${escapeHtml(vehicle.plate_no)}</strong><span>${escapeHtml(vehicle.model)} · ${escapeHtml(vehicle.energy_type)}</span></div></div></td>
      <td><span class="status-pill ${meta.className}"><i></i>${meta.label}</span></td>
      <td><div class="soc-cell"><span class="bar"><i class="${soc < 20 ? 'low' : ''}" style="width:${Math.max(2, Math.min(100, soc))}%"></i></span><b>${Math.round(soc)}%</b></div></td>
      <td><div>${escapeHtml(vehicle.driver_name || '未指派')}<div class="text-muted" style="font-size:8.5px">${escapeHtml(vehicle.driver_phone || '')}</div></div></td>
      <td class="mono">${fmtNumber(vehicle.state?.odometer_km || 0)}<span class="text-muted" style="font-size:8px"> km</span></td>
      <td><div class="terminal-bind"><span class="status-pill ${terminal?.status === 'online' ? 'status-online' : 'status-offline'}"><i></i>${terminal?.status === 'online' ? '在线' : terminal ? '离线' : '未绑定'}</span><span class="text-muted mono" style="font-size:8.5px">${escapeHtml(terminal?.phone || '')}</span></div></td>
      <td><span class="text-muted" style="font-size:9px">${timeAgo(vehicle.state?.updated_at)}</span></td>
      <td style="text-align:right"><div class="row-actions">
        <button class="icon-btn" title="实时定位" data-action="locate-vehicle" data-id="${escapeAttr(vehicle.id)}">${icon('locate')}</button>
        ${canWrite() ? `<button class="icon-btn" title="下发指令" data-action="dispatch-command" data-id="${escapeAttr(vehicle.id)}">${icon('command')}</button>` : ''}
        <button class="icon-btn" title="车辆详情" data-action="vehicle-detail" data-id="${escapeAttr(vehicle.id)}">${icon('chevron')}</button>
      </div></td>
    </tr>`;
  }).join('');

  $('#view').innerHTML = `
    <div class="page-head">
      <div class="page-head-copy"><h2>车辆管理</h2><p>管理车队档案、终端绑定与车辆实时运营状态</p></div>
      <div class="page-head-actions">
        ${canWrite() ? `<button class="btn btn-primary" data-action="add-vehicle">${icon('plus')}新增车辆</button>` : ''}
        <button class="btn" data-action="refresh">${icon('refresh')}刷新</button>
      </div>
    </div>
    <section class="panel">
      <div class="toolbar">
        <div class="toolbar-left">
          <div class="search-field">${icon('search')}<input id="vehicleSearch" type="search" placeholder="搜索车牌 / VIN / 驾驶员" value="${escapeAttr(state.vehicleSearch)}"></div>
          <select class="select" id="vehicleStatus">
            <option value="">全部状态</option>
            ${Object.entries(STATUS).map(([key, meta]) => `<option value="${key}" ${state.vehicleStatus === key ? 'selected' : ''}>${meta.label}</option>`).join('')}
          </select>
        </div>
        <div class="toolbar-right"><span class="list-summary">共 ${page.total} 台车辆</span></div>
      </div>
      <div class="table-wrap">
        <table class="data">
          <thead><tr><th>车辆</th><th>状态</th><th>电量</th><th>驾驶员</th><th>累计里程</th><th>终端</th><th>最后上报</th><th style="text-align:right">操作</th></tr></thead>
          <tbody>${rows || `<tr><td colspan="8">${emptyState('没有找到车辆', '尝试调整搜索条件或新增车辆')}</td></tr>`}</tbody>
        </table>
      </div>
      ${pagination(page, 'vehicles')}
    </section>`;

  const search = $('#vehicleSearch');
  search?.addEventListener('input', debounce(() => {
    state.vehicleSearch = search.value;
    state.vehiclePage = 1;
    loadVehicles();
  }, 320));
  $('#vehicleStatus')?.addEventListener('change', (event) => {
    state.vehicleStatus = event.target.value;
    state.vehiclePage = 1;
    loadVehicles();
  });
}

function pagination(page, scope) {
  if (!page) return '';
  const totalPages = Math.max(1, Math.ceil(page.total / page.page_size));
  const current = page.page;
  const start = Math.min(page.total, (current - 1) * page.page_size + 1);
  const end = Math.min(page.total, current * page.page_size);
  const pages = [];
  const from = Math.max(1, current - 2);
  const to = Math.min(totalPages, current + 2);
  if (from > 1) pages.push(1);
  if (from > 2) pages.push('...');
  for (let index = from; index <= to; index += 1) pages.push(index);
  if (to < totalPages - 1) pages.push('...');
  if (to < totalPages) pages.push(totalPages);
  return `<div class="pagination">
    <span>显示 ${page.total ? start : 0}–${end} 条，共 ${page.total} 条</span>
    <div class="pager">
      <button data-action="page" data-scope="${scope}" data-page="${current - 1}" ${current <= 1 ? 'disabled' : ''}>上一页</button>
      ${pages.map((item) => item === '...' ? `<span class="text-muted" style="padding:0 4px">…</span>` : `<button data-action="page" data-scope="${scope}" data-page="${item}" class="${item === current ? 'active' : ''}">${item}</button>`).join('')}
      <button data-action="page" data-scope="${scope}" data-page="${current + 1}" ${current >= totalPages ? 'disabled' : ''}>下一页</button>
    </div>
  </div>`;
}

async function openVehicleModal(id) {
  try {
    const [vehicle, page] = await Promise.all([
      api(`/vehicles/${encodeURIComponent(id)}`),
      api(`/vehicles/${encodeURIComponent(id)}/commands?page_size=5`)
    ]);
    const meta = statusOf(vehicle);
    const stateInfo = vehicle.state || {};
    const commands = page.items.map((command) => {
      const label = COMMAND_LABEL[command.command]?.label || command.command;
      const status = { success: '成功', failed: '失败', pending: '下发中', sent: '已送达', cancelled: '已取消' }[command.status] || command.status;
      return `<div class="mini-alert ${command.status === 'failed' ? 'critical' : ''}"><span class="dot" style="background:${command.status === 'success' ? '#52e0a3' : '#f6b34b'}"></span><div><strong>${escapeHtml(label)} · ${status}</strong><span>${timeAgo(command.created_at)} · ${escapeHtml(command.result || '等待终端响应')}</span></div></div>`;
    }).join('');
    openModal({
      title: '车辆档案',
      subtitle: '车辆基础信息、终端绑定与最近指令',
      size: 'modal-lg',
      body: `
        <div class="modal-vehicle-hero">
          <span class="vehicle-avatar">${icon('car')}</span>
          <div style="flex:1"><h4>${escapeHtml(vehicle.plate_no)}</h4><p>${escapeHtml(vehicle.model)} · ${escapeHtml(vehicle.color)} · ${escapeHtml(vehicle.energy_type)}</p></div>
          <span class="status-pill ${meta.className}"><i></i>${meta.label}</span>
        </div>
        <div class="modal-info-grid" style="margin-bottom:16px">
          <div><span>VIN 码</span><b class="mono">${escapeHtml(vehicle.vin)}</b></div>
          <div><span>所属组织</span><b>${escapeHtml(vehicle.org_name || '—')}</b></div>
          <div><span>驾驶员</span><b>${escapeHtml(vehicle.driver_name || '未指派')}</b></div>
          <div><span>联系电话</span><b>${escapeHtml(vehicle.driver_phone || '—')}</b></div>
          <div><span>当前里程</span><b>${fmtKm(stateInfo.odometer_km || 0)} km</b></div>
          <div><span>剩余电量</span><b>${Math.round(stateInfo.soc || 0)}%</b></div>
          <div><span>终端编号</span><b class="mono">${escapeHtml(vehicle.terminal?.phone || '未绑定')}</b></div>
          <div><span>终端状态</span><b>${vehicle.terminal ? (vehicle.terminal.status === 'online' ? '在线' : '离线') : '未绑定'}</b></div>
          <div><span>最后位置</span><b class="mono">${stateInfo.lat ? `${Number(stateInfo.lat).toFixed(5)}, ${Number(stateInfo.lon).toFixed(5)}` : '—'}</b></div>
          <div><span>最后上报</span><b>${fmtDate(stateInfo.updated_at)}</b></div>
        </div>
        <div class="detail-section-title">最近指令</div>
        ${commands || '<div class="text-muted" style="font-size:9px">暂无指令记录</div>'}
      `,
      footer: `${canWrite() ? `<button class="btn" data-action="dispatch-command" data-id="${escapeAttr(vehicle.id)}">${icon('command')}远程指令</button>` : ''}
        <button class="btn btn-primary" data-action="locate-vehicle" data-id="${escapeAttr(vehicle.id)}">${icon('locate')}实时定位</button>`
    });
  } catch (error) { toast('加载车辆失败', error.message, 'error'); }
}

function openAddVehicleModal() {
  openModal({
    title: '新增车辆',
    subtitle: '录入车辆档案，后续可绑定 T-Box 终端',
    body: `<form id="addVehicleForm" class="form-grid">
      <div class="form-field full"><label>VIN 码 <em>*</em></label><input name="vin" maxlength="17" minlength="17" placeholder="17 位车辆识别代码" required></div>
      <div class="form-field"><label>车牌号 <em>*</em></label><input name="plate_no" placeholder="沪A·D12345" required></div>
      <div class="form-field"><label>车型 <em>*</em></label><input name="model" placeholder="例如 ID.4 CROZZ" required></div>
      <div class="form-field"><label>能源类型</label><select name="energy_type"><option value="EV">纯电 EV</option><option value="PHEV">插混 PHEV</option><option value="HEV">混动 HEV</option><option value="ICE">燃油 ICE</option></select></div>
      <div class="form-field"><label>车身颜色</label><input name="color" placeholder="例如 冰川白"></div>
      <div class="form-field"><label>驾驶员</label><input name="driver_name" placeholder="驾驶员姓名"></div>
      <div class="form-field"><label>联系电话</label><input name="driver_phone" placeholder="手机号"></div>
      <div class="form-field full"><label>所属组织</label><input name="org_name" placeholder="例如 星途出行 · 浦东车队"></div>
    </form>`,
    footer: `<button class="btn" data-action="close-modal">取消</button><button class="btn btn-primary" id="saveVehicleBtn">${icon('check')}保存车辆</button>`
  });
  $('#saveVehicleBtn').addEventListener('click', async () => {
    const form = $('#addVehicleForm');
    if (!form.reportValidity()) return;
    const data = Object.fromEntries(new FormData(form).entries());
    const button = $('#saveVehicleBtn');
    button.disabled = true;
    try {
      await api('/vehicles', { method: 'POST', body: data });
      closeModal();
      toast('车辆已创建', `${data.plate_no} 已加入车队`);
      state.vehiclePage = 1;
      await loadVehicles();
    } catch (error) {
      toast('创建车辆失败', error.message, 'error');
      button.disabled = false;
    }
  });
}

/* ------------------------------------------------------------------- trips */

async function loadTrips() {
  const page = await api(`/trips?page=${state.tripPage}&page_size=12`);
  renderTrips(page);
}

function renderTrips(page) {
  const items = page.items || [];
  const completed = items.filter((trip) => trip.status === 'completed');
  const totalDistance = items.reduce((sum, trip) => sum + Number(trip.distance_km || 0), 0);
  const avgDistance = items.length ? totalDistance / items.length : 0;
  const active = items.filter((trip) => trip.status === 'active').length;
  const rows = items.map((trip) => {
    const start = trip.start_lat && trip.start_lon ? `${Number(trip.start_lat).toFixed(2)}, ${Number(trip.start_lon).toFixed(2)}` : '—';
    const end = trip.end_lat && trip.end_lon ? `${Number(trip.end_lat).toFixed(2)}, ${Number(trip.end_lon).toFixed(2)}` : '进行中';
    const duration = trip.end_ts ? (new Date(trip.end_ts) - new Date(trip.start_ts)) / 1000 : (Date.now() - new Date(trip.start_ts)) / 1000;
    return `<tr class="clickable" data-action="view-trip" data-id="${escapeAttr(trip.id)}">
      <td><div class="cell-vehicle"><span class="vehicle-avatar">${icon('route')}</span><div><strong>${escapeHtml(trip.plate_no || '—')}</strong><span>${escapeHtml(trip.model || '')}</span></div></div></td>
      <td><div class="trip-route"><div class="route-line"><i></i><span></span><i></i></div><div><strong>${escapeHtml(start)}</strong><span>${escapeHtml(end)}</span></div></div></td>
      <td>${fmtDate(trip.start_ts)}</td>
      <td class="mono">${fmtKm(trip.distance_km)} km</td>
      <td class="mono">${Math.round(trip.avg_speed)} <span class="text-muted">km/h</span></td>
      <td class="mono">${Math.round(trip.max_speed)} <span class="text-muted">km/h</span></td>
      <td>${fmtDuration(duration)}</td>
      <td><span class="command-status ${trip.status === 'completed' ? 'cmd-success' : 'cmd-sent'}"><i></i>${trip.status === 'completed' ? '已完成' : '进行中'}</span></td>
    </tr>`;
  }).join('');

  $('#view').innerHTML = `
    <div class="page-head">
      <div class="page-head-copy"><h2>行程分析</h2><p>基于 T-Box 点火与轨迹自动切分行程，沉淀里程与驾驶行为数据</p></div>
      <div class="page-head-actions"><button class="btn" data-action="refresh">${icon('refresh')}刷新数据</button></div>
    </div>
    <div class="trip-stats">
      <div class="summary-card"><span>累计行程</span><strong>${page.total}</strong></div>
      <div class="summary-card"><span>当前页里程</span><strong>${fmtKm(totalDistance)}<small style="font-size:9px;color:#5b7187"> km</small></strong></div>
      <div class="summary-card info"><span>平均单程</span><strong>${fmtKm(avgDistance)}<small style="font-size:9px;color:#5b7187"> km</small></strong></div>
      <div class="summary-card warning"><span>进行中</span><strong>${active}</strong></div>
    </div>
    <section class="panel">
      <div class="toolbar"><div class="toolbar-left"><span class="list-summary">行程按开始时间倒序展示 · 已完成 ${completed.length} 条</span></div><div class="toolbar-right"><span class="list-summary">共 ${page.total} 条</span></div></div>
      <div class="table-wrap"><table class="data">
        <thead><tr><th>车辆</th><th>起终位置</th><th>开始时间</th><th>里程</th><th>平均车速</th><th>最高车速</th><th>时长</th><th>状态</th></tr></thead>
        <tbody>${rows || `<tr><td colspan="8">${emptyState('暂无行程记录', '车辆点火行驶后将自动生成行程')}</td></tr>`}</tbody>
      </table></div>
      ${pagination(page, 'trips')}
    </section>`;
}

async function openTripModal(id) {
  try {
    const detail = await api(`/trips/${encodeURIComponent(id)}`);
    const trip = detail.trip;
    const points = detail.points || [];
    const duration = trip.end_ts ? (new Date(trip.end_ts) - new Date(trip.start_ts)) / 1000 : (Date.now() - new Date(trip.start_ts)) / 1000;
    openModal({
      title: '行程详情',
      subtitle: `${trip.plate_no || ''} · ${fmtDate(trip.start_ts)}`,
      size: 'modal-lg',
      body: `
        <div class="trip-detail-map"><canvas id="tripMap" class="map-canvas"></canvas></div>
        <div class="modal-info-grid">
          <div><span>行驶里程</span><b>${fmtKm(trip.distance_km)} km</b></div>
          <div><span>行程时长</span><b>${fmtDuration(duration)}</b></div>
          <div><span>平均车速</span><b>${Math.round(trip.avg_speed)} km/h</b></div>
          <div><span>最高车速</span><b>${Math.round(trip.max_speed)} km/h</b></div>
          <div><span>开始位置</span><b class="mono">${Number(trip.start_lat).toFixed(5)}, ${Number(trip.start_lon).toFixed(5)}</b></div>
          <div><span>结束位置</span><b class="mono">${trip.end_lat ? `${Number(trip.end_lat).toFixed(5)}, ${Number(trip.end_lon).toFixed(5)}` : '行程进行中'}</b></div>
          <div><span>轨迹点</span><b>${points.length} 个采样点</b></div>
          <div><span>行程状态</span><b>${trip.status === 'completed' ? '已完成' : '进行中'}</b></div>
        </div>`,
      footer: `<button class="btn" data-action="close-modal">关闭</button><button class="btn btn-primary" data-action="locate-vehicle" data-id="${escapeAttr(trip.vehicle_id)}">查看实时位置</button>`
    });
    setTimeout(() => {
      const canvas = $('#tripMap');
      if (!canvas) return;
      state.modalMap?.destroy();
      state.modalMap = new FleetMap(canvas, {
        vehicles: [],
        geofences: [],
        trajectory: points,
        selectedId: null,
        fitTrajectory: true
      });
    }, 40);
  } catch (error) { toast('行程详情加载失败', error.message, 'error'); }
}

/* --------------------------------------------------------------- geofences */

async function loadGeofences() {
  const [fences, vehicles] = await Promise.all([api('/geofences'), api('/vehicles/map')]);
  state.geofences = fences;
  state.vehicles = vehicles;
  renderGeofences();
}

function renderGeofences() {
  const fences = state.geofences;
  const totalInside = fences.reduce((sum, fence) => sum + Number(fence.inside_count || 0), 0);
  const active = fences.filter((fence) => fence.active).length;
  const cards = fences.map((fence) => {
    const shape = fence.kind === 'polygon' ? `${fence.polygon?.length || 0} 个顶点` : `半径 ${fmtNumber(fence.radius_m)} m`;
    return `<div class="fence-card">
      <div class="fence-card-top">
        <div class="fence-card-title"><span class="fence-color" style="background:${fence.color};color:${fence.color}"></span><strong>${escapeHtml(fence.name)}</strong></div>
        <span class="badge ${fence.active ? 'badge-live' : 'badge-muted'}">${fence.active ? '启用' : '停用'}</span>
      </div>
      <p><span class="tag">${fence.kind === 'polygon' ? '多边形' : '圆形'}</span><span>${shape}</span></p>
      <div class="fence-card-foot">
        <span class="fence-inside"><b>${fence.inside_count || 0}</b> 台车辆在围栏内</span>
        <div style="display:flex;align-items:center;gap:8px">
          ${canWrite() ? `<button class="switch ${fence.active ? 'on' : ''}" data-action="toggle-fence" data-id="${escapeAttr(fence.id)}" aria-label="启用或停用围栏"></button>` : ''}
          ${isAdmin() ? `<button class="icon-btn" style="width:24px;height:24px;border-color:transparent;background:transparent;color:#6c8398" data-action="delete-fence" data-id="${escapeAttr(fence.id)}" title="删除">${icon('x')}</button>` : ''}
        </div>
      </div>
    </div>`;
  }).join('');

  $('#view').innerHTML = `
    <div class="page-head">
      <div class="page-head-copy"><h2>电子围栏</h2><p>以地理边界定义运营区、禁停区与风险区域，自动检测进出事件</p></div>
      <div class="page-head-actions">
        ${canWrite() ? `<button class="btn btn-primary" data-action="add-geofence">${icon('plus')}新建围栏</button>` : ''}
        <button class="btn" data-action="refresh">${icon('refresh')}刷新</button>
      </div>
    </div>
    <div class="geofence-layout">
      <section class="panel geofence-map">
        <div class="panel-head">
          <div class="panel-title">${icon('fence')}<div><h3>围栏与车辆分布</h3><p>${fences.length} 个围栏 · ${totalInside} 台车辆位于围栏内</p></div></div>
          <div class="panel-tools"><span class="badge badge-live"><i></i>实时</span></div>
        </div>
        <div class="map-wrap"><canvas id="fenceMap" class="map-canvas"></canvas>
          <div class="map-overlay-top"><span class="map-chip"><span class="dot"></span>启用围栏 <b>${active}</b></span><span class="map-chip">围栏内车辆 <b>${totalInside}</b></span></div>
        </div>
        <div class="map-legend"><span><i style="background:#22d3ee"></i>运营区</span><span><i style="background:#f59e0b"></i>枢纽</span><span><i style="background:#fb7185"></i>限行区</span><span class="map-hint">点击围栏卡片可高亮定位</span></div>
      </section>
      <aside class="geofence-side" id="fenceSide">${cards || emptyState('暂无电子围栏', '创建第一个运营围栏', 'fence')}</aside>
    </div>`;

  state.map = new FleetMap($('#fenceMap'), {
    vehicles: state.vehicles,
    geofences: fences,
    onSelect: (id) => { state.selectedVehicleId = id; navigate('live', { vehicleId: id }); }
  });
  $$('.fence-card').forEach((card, index) => card.addEventListener('click', (event) => {
    if (event.target.closest('button')) return;
    state.map?.highlightFence(fences[index]?.id);
  }));
}

function openAddGeofenceModal() {
  openModal({
    title: '新建电子围栏',
    subtitle: '支持圆形缓冲区与任意多边形区域',
    body: `<form id="geofenceForm" class="form-grid">
      <div class="form-field full"><label>围栏名称 <em>*</em></label><input name="name" placeholder="例如 张江科学城" required></div>
      <div class="form-field"><label>围栏类型</label><select name="kind" id="fenceKind"><option value="circle">圆形围栏</option><option value="polygon">多边形围栏</option></select></div>
      <div class="form-field"><label>标识颜色</label><input name="color" type="color" value="#22d3ee" style="padding:5px"></div>
      <div class="form-field" id="centerLatField"><label>中心纬度</label><input name="center_lat" type="number" step="0.000001" value="31.230400"></div>
      <div class="form-field" id="centerLonField"><label>中心经度</label><input name="center_lon" type="number" step="0.000001" value="121.473700"></div>
      <div class="form-field" id="radiusField"><label>半径（米）</label><input name="radius_m" type="number" min="50" max="50000" value="800"></div>
      <div class="form-field full hidden" id="polygonField"><label>多边形顶点</label><textarea name="polygon" placeholder="经度,纬度; 经度,纬度; … 至少 3 个点">121.40,31.20; 121.50,31.20; 121.50,31.30; 121.40,31.30</textarea><small>格式：经度,纬度，使用英文分号分隔顶点</small></div>
    </form>`,
    footer: `<button class="btn" data-action="close-modal">取消</button><button class="btn btn-primary" id="saveFenceBtn">${icon('check')}创建围栏</button>`
  });
  $('#fenceKind').addEventListener('change', (event) => {
    const poly = event.target.value === 'polygon';
    $('#centerLatField').classList.toggle('hidden', poly);
    $('#centerLonField').classList.toggle('hidden', poly);
    $('#radiusField').classList.toggle('hidden', poly);
    $('#polygonField').classList.toggle('hidden', !poly);
  });
  $('#saveFenceBtn').addEventListener('click', async () => {
    const form = $('#geofenceForm');
    if (!form.reportValidity()) return;
    const data = Object.fromEntries(new FormData(form).entries());
    const body = {
      name: data.name,
      kind: data.kind,
      color: data.color,
      radius_m: Number(data.radius_m) || 800,
      active: true
    };
    if (data.kind === 'polygon') {
      const points = data.polygon.split(';').map((pair) => pair.trim()).filter(Boolean).map((pair) => {
        const [lon, lat] = pair.split(',').map(Number);
        return [lon, lat];
      });
      if (points.length < 3 || points.some(([lon, lat]) => !Number.isFinite(lon) || !Number.isFinite(lat))) {
        toast('顶点格式不正确', '请按“经度,纬度”输入至少 3 个顶点', 'error');
        return;
      }
      body.polygon = points;
    } else {
      body.center_lat = Number(data.center_lat);
      body.center_lon = Number(data.center_lon);
    }
    const button = $('#saveFenceBtn');
    button.disabled = true;
    try {
      await api('/geofences', { method: 'POST', body });
      closeModal();
      toast('围栏已创建', body.name);
      await loadGeofences();
    } catch (error) {
      toast('创建围栏失败', error.message, 'error');
      button.disabled = false;
    }
  });
}

async function toggleFence(id, active) {
  try {
    await api(`/geofences/${encodeURIComponent(id)}`, { method: 'PATCH', body: { active: !active } });
    toast('围栏状态已更新', !active ? '围栏已启用' : '围栏已停用');
    await loadGeofences();
  } catch (error) { toast('更新失败', error.message, 'error'); }
}

/* ------------------------------------------------------------------ alerts */

async function loadAlerts() {
  const query = new URLSearchParams({ page: String(state.alertPage), page_size: '10' });
  if (state.alertFilter) query.set('status', state.alertFilter);
  if (state.alertLevel) query.set('level', state.alertLevel);
  const [page, dashboard] = await Promise.all([api(`/alerts?${query}`), api('/dashboard')]);
  state.alerts = page;
  state.dashboard = dashboard;
  state.alertUnread = dashboard.alerts.active;
  updateAlertBadge();
  renderAlerts(page, dashboard.alerts);
}

function renderAlerts(page, stats) {
  const rows = page.items.map((alert) => {
    const level = LEVEL[alert.level] || LEVEL.info;
    const meta = ALERT_META[alert.alert_type] || { label: '告警', icon: 'bell' };
    return `<tr>
      <td><span class="badge ${level.badge}">${icon(level.icon)}${level.label}</span></td>
      <td><div class="alert-table-title"><span class="ico alert-ico ${alert.level}">${icon(meta.icon)}</span><div><strong>${escapeHtml(alert.title)}</strong><span>${escapeHtml(alert.message)}</span></div></div></td>
      <td><div class="cell-vehicle"><div><strong>${escapeHtml(alert.plate_no || '—')}</strong><span>${escapeHtml(alert.model || '')}</span></div></div></td>
      <td><span class="text-muted">${fmtDate(alert.ts)}</span><div class="text-muted" style="font-size:8px">${timeAgo(alert.ts)}</div></td>
      <td>${alert.acknowledged ? `<span class="ack-state done">已确认<br><span style="font-size:8px;color:#5b7187">${escapeHtml(alert.ack_by || '')} · ${fmtDate(alert.ack_at)}</span></span>` : '<span class="ack-state">待处置</span>'}</td>
      <td style="text-align:right">${!alert.acknowledged && canWrite() ? `<button class="btn btn-sm" data-action="ack-alert" data-id="${escapeAttr(alert.id)}">${icon('check')}确认</button>` : '<span class="text-muted" style="font-size:9px">—</span>'}</td>
    </tr>`;
  }).join('');

  $('#view').innerHTML = `
    <div class="page-head">
      <div class="page-head-copy"><h2>告警中心</h2><p>聚合超速、低电量、碰撞与围栏事件，形成可闭环的风险处置流程</p></div>
      <div class="page-head-actions">
        ${stats.active > 0 && canWrite() ? `<button class="btn" data-action="ack-all">${icon('check')}全部确认</button>` : ''}
        <button class="btn" data-action="refresh">${icon('refresh')}刷新</button>
      </div>
    </div>
    <div class="alert-summary">
      <div class="summary-card critical"><span>活跃告警</span><strong>${stats.active}</strong></div>
      <div class="summary-card critical"><span>严重等级</span><strong>${stats.critical}</strong></div>
      <div class="summary-card warning"><span>警告等级</span><strong>${stats.warning}</strong></div>
      <div class="summary-card info"><span>今日新增</span><strong>${stats.today}</strong></div>
    </div>
    <section class="panel">
      <div class="toolbar">
        <div class="toolbar-left"><div class="segmented" id="alertFilters">
          <button data-status="active" class="${state.alertFilter === 'active' ? 'active' : ''}">待处置</button>
          <button data-status="acknowledged" class="${state.alertFilter === 'acknowledged' ? 'active' : ''}">已确认</button>
          <button data-status="" class="${state.alertFilter === '' ? 'active' : ''}">全部</button>
        </div>
        <select class="select" id="alertLevel">
          <option value="">全部等级</option>
          <option value="critical" ${state.alertLevel === 'critical' ? 'selected' : ''}>严重</option>
          <option value="warning" ${state.alertLevel === 'warning' ? 'selected' : ''}>警告</option>
          <option value="info" ${state.alertLevel === 'info' ? 'selected' : ''}>提示</option>
        </select></div>
        <div class="toolbar-right"><span class="list-summary">共 ${page.total} 条记录</span></div>
      </div>
      <div class="table-wrap"><table class="data">
        <thead><tr><th>等级</th><th>告警事件</th><th>车辆</th><th>发生时间</th><th>处置状态</th><th style="text-align:right">操作</th></tr></thead>
        <tbody>${rows || `<tr><td colspan="6">${emptyState('暂无告警', '当前筛选条件下没有风险事件', 'shield')}</td></tr>`}</tbody>
      </table></div>
      ${pagination(page, 'alerts')}
    </section>`;

  $$('#alertFilters button').forEach((button) => button.addEventListener('click', () => {
    state.alertFilter = button.dataset.status;
    state.alertPage = 1;
    loadAlerts();
  }));
  $('#alertLevel')?.addEventListener('change', (event) => {
    state.alertLevel = event.target.value;
    state.alertPage = 1;
    loadAlerts();
  });
}

async function acknowledgeAlert(id) {
  try {
    await api(`/alerts/${encodeURIComponent(id)}/ack`, { method: 'POST', body: {} });
    toast('告警已确认', '风险事件已进入处置记录');
    await loadAlerts();
  } catch (error) { toast('确认失败', error.message, 'error'); }
}

async function acknowledgeAll() {
  const ok = await confirmDialog({
    title: '确认全部告警？',
    message: '所有待处置告警将被标记为已确认，操作会记录到审计日志。',
    confirmText: '全部确认'
  });
  if (!ok) return;
  try {
    const result = await api('/alerts/ack-all', { method: 'POST', body: {} });
    toast('批量确认完成', `已确认 ${result.acknowledged} 条告警`);
    await loadAlerts();
  } catch (error) { toast('批量确认失败', error.message, 'error'); }
}

/* ---------------------------------------------------------------- commands */

async function loadCommands() {
  const [vehicles, commands] = await Promise.all([
    api('/vehicles?page_size=200'),
    api('/commands?page_size=12')
  ]);
  state.allVehicles = vehicles.items || [];
  state.commands = commands;
  const selectable = state.allVehicles.filter((vehicle) => !['offline', 'maintenance'].includes(vehicle.state?.status));
  if (!state.allVehicles.find((vehicle) => vehicle.id === state.commandVehicleId)) {
    state.commandVehicleId = (selectable[0] || state.allVehicles[0])?.id || null;
  }
  renderCommands();
}

function renderCommands() {
  const vehicles = state.allVehicles;
  const commands = state.commands?.items || [];
  const selectedVehicle = vehicles.find((vehicle) => vehicle.id === state.commandVehicleId);
  const selectedMeta = selectedVehicle ? statusOf(selectedVehicle) : null;
  const history = commands.map((command) => {
    const meta = COMMAND_LABEL[command.command] || { label: command.command, icon: 'command' };
    const statusText = { success: '执行成功', failed: '执行失败', pending: '等待下发', sent: '已送达', cancelled: '已取消' }[command.status] || command.status;
    const statusClass = { success: 'cmd-success', failed: 'cmd-failed', pending: 'cmd-pending', sent: 'cmd-sent', cancelled: 'cmd-cancelled' }[command.status] || '';
    return `<div class="timeline-row">
      <span class="timeline-ico">${icon(meta.icon)}</span>
      <div class="timeline-main">
        <div class="timeline-top"><strong>${escapeHtml(meta.label)} <span class="text-muted" style="font-weight:400">· ${escapeHtml(command.plate_no || '')}</span></strong><time>${timeAgo(command.created_at)}</time></div>
        <div class="timeline-sub">${escapeHtml(command.requested_by || '')} 下发 · <span class="command-status ${statusClass}"><i></i>${statusText}</span></div>
        <div class="timeline-result">${escapeHtml(command.result || '等待终端响应…')}</div>
      </div>
    </div>`;
  }).join('');

  $('#view').innerHTML = `
    <div class="page-head">
      <div class="page-head-copy"><h2>指令中心</h2><p>通过 T-Box 安全通道下发车控指令，实时跟踪执行结果</p></div>
      <div class="page-head-actions"><span class="badge badge-muted">${icon('shield')}双向认证 · 指令审计</span><button class="btn" data-action="refresh">${icon('refresh')}刷新</button></div>
    </div>
    <div class="command-layout">
      <section class="panel command-card">
        <h3>下发远程指令</h3>
        <p>选择目标车辆与指令类型，指令将经平台安全域校验后下发</p>
        <div class="command-vehicle">
          <label>目标车辆</label>
          <select id="commandVehicle">
            ${vehicles.map((vehicle) => {
              const meta = statusOf(vehicle);
              const disabled = ['offline', 'maintenance'].includes(vehicle.state?.status);
              return `<option value="${escapeAttr(vehicle.id)}" ${vehicle.id === state.commandVehicleId ? 'selected' : ''} ${disabled ? 'disabled' : ''}>${escapeHtml(vehicle.plate_no)} · ${escapeHtml(vehicle.model)}（${meta.label}）</option>`;
            }).join('')}
          </select>
          ${selectedVehicle ? `<div style="display:flex;align-items:center;gap:8px;margin-top:8px"><span class="status-pill ${selectedMeta.className}"><i></i>${selectedMeta.label}</span><span class="text-muted" style="font-size:8.5px">SOC ${Math.round(socOf(selectedVehicle))}% · ${escapeHtml(selectedVehicle.driver_name || '未指派')}</span></div>` : ''}
        </div>
        <label style="display:block;color:#8da2b6;font-size:9.5px;margin-bottom:7px">指令类型</label>
        <div class="command-grid" id="commandGrid">
          ${COMMANDS.map((command) => `<button class="command-tile ${state.commandType === command.key ? 'active' : ''} ${command.danger ? 'danger' : ''}" data-command="${command.key}">${icon(command.icon)}<span>${command.label}</span></button>`).join('')}
        </div>
        <div class="command-note">${icon('shield')}<span>远程启动、车窗与充电类指令属于安全敏感操作，将记录操作人、车辆与终端响应。</span></div>
        <button class="btn btn-primary command-submit" data-action="send-command" ${!canWrite() || !selectedVehicle ? 'disabled' : ''}>${icon('command')}下发指令</button>
      </section>
      <section class="panel command-history">
        <div class="panel-head"><div class="panel-title">${icon('clock')}<div><h3>指令执行记录</h3><p>最近 12 条 · 实时回传终端状态</p></div></div><span class="badge badge-live"><i></i>LIVE</span></div>
        <div class="timeline">${history || emptyState('暂无指令记录', '下发第一条远程指令', 'command')}</div>
      </section>
    </div>`;

  $('#commandVehicle')?.addEventListener('change', (event) => {
    state.commandVehicleId = event.target.value;
    renderCommands();
  });
  $$('#commandGrid .command-tile').forEach((tile) => tile.addEventListener('click', () => {
    state.commandType = tile.dataset.command;
    $$('#commandGrid .command-tile').forEach((item) => item.classList.toggle('active', item === tile));
  }));
}

async function sendCommand(vehicleId, command, options = {}) {
  const vehicle = state.allVehicles.find((item) => item.id === vehicleId);
  const label = COMMAND_LABEL[command]?.label || command;
  if (!vehicle) { toast('车辆不存在', '请刷新后重试', 'error'); return false; }
  if (!options.skipConfirm) {
    const ok = await confirmDialog({
      title: `确认下发「${label}」？`,
      message: `目标车辆：${vehicle.plate_no}（${vehicle.model}）。指令下发后将立即尝试通过 T-Box 执行，请确认车辆处于安全状态。`,
      confirmText: '确认下发'
    });
    if (!ok) return false;
  }
  try {
    await api(`/vehicles/${encodeURIComponent(vehicleId)}/commands`, {
      method: 'POST',
      body: { command, payload: null }
    });
    toast('指令已下发', `${vehicle.plate_no} · ${label}，等待终端响应`);
    if (state.view === 'commands') {
      clearTimeout(state.refreshTimer);
      state.refreshTimer = setTimeout(loadCommands, 600);
    }
    return true;
  } catch (error) {
    toast('指令下发失败', error.message, 'error');
    return false;
  }
}

async function openCommandModal(id) {
  let vehicle = state.allVehicles.find((item) => item.id === id) || state.vehicles.find((item) => item.id === id);
  if (!vehicle) {
    try {
      vehicle = await api(`/vehicles/${encodeURIComponent(id)}`);
      state.allVehicles.push(vehicle);
      state.vehicles.push(vehicle);
    } catch (error) {
      toast('车辆信息加载失败', error.message, 'error');
      return;
    }
  }
  let selected = 'lock';
  openModal({
    title: '远程指令',
    subtitle: `${vehicle.plate_no} · ${vehicle.model}`,
    body: `<div class="command-vehicle" style="margin-bottom:14px"><div style="display:flex;align-items:center;gap:9px"><span class="vehicle-avatar">${icon('car')}</span><div><strong class="mono" style="font-size:13px;font-weight:500">${escapeHtml(vehicle.plate_no)}</strong><div class="text-muted" style="font-size:8.5px">${escapeHtml(vehicle.driver_name || '未指派')} · SOC ${Math.round(socOf(vehicle))}%</div></div></div></div>
      <div class="command-grid">
        ${COMMANDS.map((command) => `<button class="command-tile ${command.key === selected ? 'active' : ''} ${command.danger ? 'danger' : ''}" data-command="${command.key}">${icon(command.icon)}<span>${command.label}</span></button>`).join('')}
      </div>
      <div class="command-note" style="margin-top:14px">${icon('shield')}<span>指令将记录在审计日志中。请确保车辆处于安全环境，避免误操作。</span></div>`,
    footer: `<button class="btn" data-action="close-modal">取消</button><button class="btn btn-primary" id="modalSendCommand">${icon('command')}确认下发</button>`
  });
  $$('#modalRoot .command-tile').forEach((tile) => tile.addEventListener('click', () => {
    selected = tile.dataset.command;
    $$('#modalRoot .command-tile').forEach((item) => item.classList.toggle('active', item === tile));
  }));
  $('#modalSendCommand').addEventListener('click', async () => {
    const ok = await sendCommand(vehicle.id, selected, { skipConfirm: true });
    if (ok) closeModal();
  });
}

/* --------------------------------------------------------------- terminals */

async function loadTerminals() {
  const [terminals, vehicles] = await Promise.all([api('/terminals'), api('/vehicles?page_size=200')]);
  state.terminals = terminals;
  state.allVehicles = vehicles.items || [];
  renderTerminals();
}

function renderTerminals() {
  const terminals = state.terminals;
  const total = terminals.length;
  const online = terminals.filter((terminal) => terminal.status === 'online').length;
  const bound = terminals.filter((terminal) => terminal.vehicle_id).length;
  const offline = total - online;
  const rows = terminals.map((terminal) => `<tr>
    <td><div class="cell-vehicle"><span class="vehicle-avatar">${icon('chip')}</span><div><strong>${escapeHtml(terminal.phone || terminal.imei)}</strong><span class="mono">IMEI ${escapeHtml(terminal.imei)}</span></div></div></td>
    <td><span class="status-pill ${terminal.status === 'online' ? 'status-online' : 'status-offline'}"><i></i>${terminal.status === 'online' ? '在线' : '离线'}</span></td>
    <td><div>${escapeHtml(terminal.model || '—')}<div class="text-muted mono" style="font-size:8.5px">${escapeHtml(terminal.protocol || '')}</div></div></td>
    <td>${terminal.plate_no ? `<button class="panel-link" data-action="locate-vehicle" data-id="${escapeAttr(terminal.vehicle_id)}">${escapeHtml(terminal.plate_no)} ${icon('chevron')}</button>` : '<span class="text-muted" style="font-size:9px">未绑定</span>'}</td>
    <td class="mono" style="font-size:9px">${escapeHtml(terminal.iccid || '—')}</td>
    <td><span class="text-muted" style="font-size:9px">${timeAgo(terminal.last_heartbeat_at)}</span></td>
    <td style="text-align:right">${canWrite() ? (terminal.vehicle_id
      ? `<button class="btn btn-sm" data-action="bind-terminal" data-id="${escapeAttr(terminal.id)}">${icon('link')}更换绑定</button>`
      : `<button class="btn btn-sm" data-action="bind-terminal" data-id="${escapeAttr(terminal.id)}">${icon('plus')}绑定车辆</button>`) : '<span class="text-muted">—</span>'}</td>
  </tr>`).join('');

  $('#view').innerHTML = `
    <div class="page-head">
      <div class="page-head-copy"><h2>终端管理</h2><p>管理 T-Box 接入、物联网卡与车辆绑定关系，监控心跳健康度</p></div>
      <div class="page-head-actions">
        ${canWrite() ? `<button class="btn btn-primary" data-action="add-terminal">${icon('plus')}新增终端</button>` : ''}
        <button class="btn" data-action="refresh">${icon('refresh')}刷新</button>
      </div>
    </div>
    <div class="terminal-summary">
      <div class="summary-card"><span>终端总数</span><strong>${total}</strong></div>
      <div class="summary-card info"><span>在线终端</span><strong>${online}</strong></div>
      <div class="summary-card warning"><span>离线终端</span><strong>${offline}</strong></div>
      <div class="summary-card"><span>已绑定车辆</span><strong>${bound}</strong></div>
    </div>
    <section class="panel">
      <div class="toolbar"><div class="toolbar-left"><span class="list-summary">JT/T 808 接入 · 心跳周期 30s · 离线判定 90s</span></div><div class="toolbar-right"><span class="list-summary">在线率 ${total ? ((online / total) * 100).toFixed(1) : '0.0'}%</span></div></div>
      <div class="table-wrap"><table class="data">
        <thead><tr><th>终端 / 终端号</th><th>状态</th><th>型号 / 协议</th><th>绑定车辆</th><th>ICCID</th><th>最后心跳</th><th style="text-align:right">操作</th></tr></thead>
        <tbody>${rows || `<tr><td colspan="7">${emptyState('暂无终端', '接入 T-Box 后将显示在这里', 'chip')}</td></tr>`}</tbody>
      </table></div>
    </section>`;
}

function openAddTerminalModal() {
  const candidates = state.allVehicles.filter((vehicle) => !vehicle.terminal);
  const options = candidates.map((vehicle) => `<option value="${escapeAttr(vehicle.id)}">${escapeHtml(vehicle.plate_no)} · ${escapeHtml(vehicle.model)}</option>`).join('');
  openModal({
    title: '新增 T-Box 终端',
    subtitle: '创建终端并绑定车辆，终端号用于 JT/T 808 鉴权',
    body: `<form id="addTerminalForm" class="form-grid">
      <div class="form-field full"><label>绑定车辆 <em>*</em></label><select name="vehicle_id" required>${options || '<option value="">暂无可绑定车辆</option>'}</select><small>仅显示尚未绑定终端的车辆。</small></div>
      <div class="form-field"><label>终端号（12 位）</label><input name="phone" maxlength="12" placeholder="013800000019"><small>JT/T 808 终端手机号</small></div>
      <div class="form-field"><label>IMEI</label><input name="imei" placeholder="861234567890019" required></div>
      <div class="form-field"><label>ICCID</label><input name="iccid" placeholder="89860012345678900019"></div>
      <div class="form-field"><label>终端型号</label><input name="model" value="XT-TBOX-5G"></div>
    </form>`,
    footer: `<button class="btn" data-action="close-modal">取消</button><button class="btn btn-primary" id="saveTerminalBtn" ${candidates.length ? '' : 'disabled'}>${icon('link')}创建并绑定</button>`
  });
  $('#saveTerminalBtn')?.addEventListener('click', async () => {
    const form = $('#addTerminalForm');
    if (!form.reportValidity()) return;
    const data = Object.fromEntries(new FormData(form).entries());
    if (!data.vehicle_id) { toast('没有可绑定车辆', '请先新增车辆或解绑已有终端', 'warning'); return; }
    if (data.phone && !/^\d{12}$/.test(data.phone)) { toast('终端号格式不正确', '终端号必须为 12 位数字', 'error'); return; }
    const button = $('#saveTerminalBtn');
    button.disabled = true;
    try {
      await api(`/vehicles/${encodeURIComponent(data.vehicle_id)}/terminal`, {
        method: 'POST',
        body: { imei: data.imei, phone: data.phone, iccid: data.iccid, model: data.model }
      });
      closeModal();
      toast('终端已接入', `${data.imei} 已绑定车辆`);
      await loadTerminals();
    } catch (error) {
      toast('终端创建失败', error.message, 'error');
      button.disabled = false;
    }
  });
}

function openBindTerminalModal(terminalId) {  const terminal = state.terminals.find((item) => item.id === terminalId);
  if (!terminal) return;
  const options = state.allVehicles.map((vehicle) => `<option value="${escapeAttr(vehicle.id)}" ${vehicle.id === terminal.vehicle_id ? 'selected' : ''}>${escapeHtml(vehicle.plate_no)} · ${escapeHtml(vehicle.model)}</option>`).join('');
  openModal({
    title: terminal.vehicle_id ? '更换终端绑定' : '绑定终端到车辆',
    subtitle: `终端 ${terminal.phone || terminal.imei} · ${terminal.model || ''}`,
    body: `<div class="form-field"><label>目标车辆</label><select id="bindVehicle"><option value="">解除绑定</option>${options}</select><small>一辆车只能绑定一个终端，若目标车辆已绑定其他终端需要先解绑。</small></div>`,
    footer: `<button class="btn" data-action="close-modal">取消</button><button class="btn btn-primary" id="confirmBind">${icon('link')}保存绑定</button>`
  });
  $('#confirmBind').addEventListener('click', async () => {
    const vehicleId = $('#bindVehicle').value || null;
    try {
      await api(`/terminals/${encodeURIComponent(terminalId)}/bind`, { method: 'POST', body: { vehicle_id: vehicleId } });
      closeModal();
      toast('绑定关系已更新', vehicleId ? '终端已绑定车辆' : '终端已解除绑定');
      await loadTerminals();
    } catch (error) { toast('绑定失败', error.message, 'error'); }
  });
}

/* ------------------------------------------------------------------- modal */

function openModal({ title, subtitle = '', body, footer = '', size = '' }) {
  state.modalResolver = null;
  $('#modalRoot').innerHTML = `
    <div class="modal-backdrop" data-action="close-modal"></div>
    <div class="modal ${size}" role="dialog" aria-modal="true">
      <div class="modal-head">
        <div><h3>${escapeHtml(title)}</h3>${subtitle ? `<p>${escapeHtml(subtitle)}</p>` : ''}</div>
        <button class="modal-close" data-action="close-modal" aria-label="关闭">${icon('close')}</button>
      </div>
      <div class="modal-body">${body}</div>
      ${footer ? `<div class="modal-foot">${footer}</div>` : ''}
    </div>`;
  $('#modalRoot').classList.add('open');
  document.body.style.overflow = 'hidden';
}

function closeModal() {
  if (state.modalResolver) {
    const resolve = state.modalResolver;
    state.modalResolver = null;
    resolve(false);
  }
  state.modalMap?.destroy();
  state.modalMap = null;
  $('#modalRoot').classList.remove('open');
  $('#modalRoot').innerHTML = '';
  document.body.style.overflow = '';
}

function confirmDialog({ title, message, confirmText = '确认', danger = false }) {
  return new Promise((resolve) => {
    openModal({
      title,
      size: 'modal-sm',
      body: `<div class="confirm-icon">${icon(danger ? 'alert' : 'info')}</div><div class="confirm-text">${message}</div>`,
      footer: `<button class="btn" id="confirmCancel">取消</button><button class="btn ${danger ? 'btn-danger' : 'btn-primary'}" id="confirmYes">${escapeHtml(confirmText)}</button>`
    });
    state.modalResolver = (value) => resolve(value);
    $('#confirmCancel').addEventListener('click', () => closeModal());
    $('#confirmYes').addEventListener('click', () => {
      const resolver = state.modalResolver;
      state.modalResolver = null;
      resolver?.(true);
      $('#modalRoot').classList.remove('open');
      $('#modalRoot').innerHTML = '';
      document.body.style.overflow = '';
    });
  });
}

/* ---------------------------------------------------------------- map core */

class FleetMap {
  constructor(canvas, options = {}) {
    this.canvas = canvas;
    this.ctx = canvas.getContext('2d');
    this.wrap = canvas.parentElement;
    this.vehicles = options.vehicles || [];
    this.geofences = options.geofences || [];
    this.trajectory = options.trajectory || [];
    this.selectedId = options.selectedId || null;
    this.highlightFenceId = null;
    this.onSelect = options.onSelect || (() => {});
    this.fitTrajectory = !!options.fitTrajectory;
    this.bounds = { minLon: 120.98, maxLon: 122.05, minLat: 30.70, maxLat: 31.78 };
    this.hoverId = null;
    this.tooltip = document.createElement('div');
    this.tooltip.className = 'map-tooltip';
    this.tooltip.style.display = 'none';
    this.wrap.appendChild(this.tooltip);
    this.timer = null;
    this.screen = new Map();
    this.pulse = 0;
    this._onMove = this.handleMove.bind(this);
    this._onLeave = () => { this.hoverId = null; this.tooltip.style.display = 'none'; };
    this._onClick = this.handleClick.bind(this);
    this._onResize = () => this.draw();
    canvas.addEventListener('mousemove', this._onMove);
    canvas.addEventListener('mouseleave', this._onLeave);
    canvas.addEventListener('click', this._onClick);
    this.resizeObserver = new ResizeObserver(this._onResize);
    this.resizeObserver.observe(this.wrap);
    this.timer = setInterval(() => { this.pulse = (this.pulse + 0.018) % 1; this.draw(); }, 90);
    this.draw();
  }

  setData({ vehicles = this.vehicles, geofences = this.geofences, trajectory = this.trajectory, selectedId = this.selectedId }) {
    this.vehicles = vehicles;
    this.geofences = geofences;
    this.trajectory = trajectory;
    this.selectedId = selectedId;
    this.draw();
  }

  setTrajectory(points) { this.trajectory = points; this.draw(); }

  highlightFence(id) {
    this.highlightFenceId = this.highlightFenceId === id ? null : id;
    this.draw();
  }

  destroy() {
    clearInterval(this.timer);
    this.resizeObserver?.disconnect();
    this.canvas.removeEventListener('mousemove', this._onMove);
    this.canvas.removeEventListener('mouseleave', this._onLeave);
    this.canvas.removeEventListener('click', this._onClick);
    this.tooltip.remove();
  }

  project(lat, lon, width, height, pad = 24) {
    const { minLon, maxLon, minLat, maxLat } = this.bounds;
    const x = pad + ((Number(lon) - minLon) / (maxLon - minLon)) * (width - pad * 2);
    const y = pad + ((maxLat - Number(lat)) / (maxLat - minLat)) * (height - pad * 2);
    return { x, y };
  }

  zoomForTrajectory() {
    const points = this.trajectory.filter((point) => Number.isFinite(Number(point.lat)) && Number.isFinite(Number(point.lon)));
    if (points.length < 2) return this.bounds;
    const lats = points.map((point) => Number(point.lat));
    const lons = points.map((point) => Number(point.lon));
    const minLat = Math.min(...lats), maxLat = Math.max(...lats);
    const minLon = Math.min(...lons), maxLon = Math.max(...lons);
    const latPad = Math.max(0.008, (maxLat - minLat) * 0.25);
    const lonPad = Math.max(0.008, (maxLon - minLon) * 0.25);
    return {
      minLat: minLat - latPad, maxLat: maxLat + latPad,
      minLon: minLon - lonPad, maxLon: maxLon + lonPad
    };
  }

  drawBackground(width, height) {
    const ctx = this.ctx;
    const bg = ctx.createLinearGradient(0, 0, width, height);
    bg.addColorStop(0, '#07131f');
    bg.addColorStop(.55, '#081624');
    bg.addColorStop(1, '#07111d');
    ctx.fillStyle = bg;
    ctx.fillRect(0, 0, width, height);

    const glow = ctx.createRadialGradient(width * .58, height * .42, 0, width * .58, height * .42, Math.max(width, height) * .55);
    glow.addColorStop(0, 'rgba(35, 119, 126, .12)');
    glow.addColorStop(.55, 'rgba(23, 79, 107, .05)');
    glow.addColorStop(1, 'rgba(0, 0, 0, 0)');
    ctx.fillStyle = glow;
    ctx.fillRect(0, 0, width, height);

    ctx.strokeStyle = 'rgba(112, 153, 181, .07)';
    ctx.lineWidth = 1;
    for (let index = 0; index <= 10; index += 1) {
      const x = (width / 10) * index;
      const y = (height / 8) * index;
      ctx.beginPath(); ctx.moveTo(x, 0); ctx.lineTo(x, height); ctx.stroke();
      if (index <= 8) { ctx.beginPath(); ctx.moveTo(0, y); ctx.lineTo(width, y); ctx.stroke(); }
    }

    const project = (lat, lon) => this.project(lat, lon, width, height);
    const polygon = (points, fill, stroke, dash = []) => {
      ctx.beginPath();
      points.forEach(([lat, lon], index) => {
        const point = project(lat, lon);
        if (index === 0) ctx.moveTo(point.x, point.y); else ctx.lineTo(point.x, point.y);
      });
      ctx.closePath();
      if (fill) { ctx.fillStyle = fill; ctx.fill(); }
      if (stroke) { ctx.strokeStyle = stroke; ctx.lineWidth = 1; ctx.setLineDash(dash); ctx.stroke(); ctx.setLineDash([]); }
    };
    const line = (points, stroke, widthValue = 1, dash = []) => {
      ctx.beginPath();
      points.forEach(([lat, lon], index) => {
        const point = project(lat, lon);
        if (index === 0) ctx.moveTo(point.x, point.y); else ctx.lineTo(point.x, point.y);
      });
      ctx.strokeStyle = stroke;
      ctx.lineWidth = widthValue;
      ctx.setLineDash(dash);
      ctx.stroke();
      ctx.setLineDash([]);
    };

    polygon([
      [31.45, 121.00], [31.55, 121.10], [31.60, 121.25], [31.70, 121.35], [31.68, 121.50],
      [31.62, 121.65], [31.55, 121.85], [31.40, 121.95], [31.25, 122.02], [31.05, 121.98],
      [30.95, 121.85], [30.85, 121.75], [30.78, 121.55], [30.82, 121.40], [30.90, 121.30],
      [31.00, 121.20], [31.10, 121.10], [31.20, 121.00]
    ], 'rgba(29, 71, 88, .25)', 'rgba(90, 154, 176, .3)');

    polygon([
      [31.72, 121.15], [31.78, 121.35], [31.76, 121.65], [31.68, 121.85], [31.60, 121.70],
      [31.62, 121.45], [31.65, 121.25]
    ], 'rgba(29, 71, 88, .19)', 'rgba(90, 154, 176, .22)');

    const river = [
      [31.35, 121.42], [31.30, 121.46], [31.27, 121.49], [31.23, 121.50], [31.20, 121.48],
      [31.16, 121.46], [31.12, 121.47], [31.08, 121.51], [31.03, 121.54]
    ];
    line(river, 'rgba(23, 63, 89, .9)', 12);
    line(river, 'rgba(43, 113, 132, .28)', 3);
    line([[31.26, 121.31], [31.25, 121.38], [31.24, 121.44], [31.25, 121.50]], 'rgba(36, 87, 108, .5)', 5);

    line([[31.30, 121.32], [31.36, 121.45], [31.31, 121.61], [31.20, 121.68], [31.09, 121.58], [31.10, 121.42], [31.19, 121.30]], 'rgba(82, 126, 147, .3)', 2, [4, 5]);
    line([[31.27, 121.40], [31.28, 121.49], [31.23, 121.54], [31.17, 121.52], [31.16, 121.44], [31.21, 121.39]], 'rgba(82, 126, 147, .32)', 1.5);
    line([[31.23, 121.47], [31.36, 121.25]], 'rgba(82, 126, 147, .3)', 1.5);
    line([[31.23, 121.47], [31.03, 121.23]], 'rgba(82, 126, 147, .3)', 1.5);
    line([[31.23, 121.47], [31.14, 121.81]], 'rgba(82, 126, 147, .35)', 2);
    line([[31.23, 121.47], [31.38, 121.48], [31.48, 121.49]], 'rgba(82, 126, 147, .3)', 1.5);
    line([[31.23, 121.47], [30.90, 121.88]], 'rgba(82, 126, 147, .28)', 1.5);
    line([[31.23, 121.47], [31.63, 121.40]], 'rgba(82, 126, 147, .2)', 1.5, [3, 4]);
    line([[31.36, 121.25], [31.40, 121.10], [31.45, 120.99]], 'rgba(82, 126, 147, .2)', 1.2);

    ctx.font = '10px "Segoe UI", "Microsoft YaHei", sans-serif';
    ctx.fillStyle = 'rgba(128, 165, 187, .34)';
    ctx.textAlign = 'center';
    const labels = [
      [31.66, 121.40, '崇明'], [31.39, 121.25, '嘉定'], [31.40, 121.49, '宝山'],
      [31.15, 121.11, '青浦'], [31.03, 121.23, '松江'], [31.11, 121.38, '闵行'],
      [31.13, 121.55, '浦东新区'], [30.92, 121.47, '奉贤'], [30.76, 121.34, '金山'],
      [31.23, 121.47, '市中心'], [31.16, 121.80, '浦东机场']
    ];
    labels.forEach(([lat, lon, text]) => {
      const point = project(lat, lon);
      ctx.fillText(text, point.x, point.y);
    });
    ctx.fillStyle = 'rgba(78, 111, 132, .1)';
    ctx.font = '600 25px "Segoe UI", sans-serif';
    ctx.textAlign = 'left';
    ctx.fillText('SHANGHAI', 18, height - 16);
    ctx.font = '9px "Segoe UI", sans-serif';
    ctx.fillStyle = 'rgba(78, 111, 132, .28)';
    ctx.fillText('31.2304° N  ·  121.4737° E', 18, height - 30);
  }

  drawFences(width, height) {
    const ctx = this.ctx;
    this.geofences.forEach((fence) => {
      const highlighted = this.highlightFenceId === fence.id;
      const alpha = fence.active ? (highlighted ? .3 : .13) : .04;
      ctx.save();
      if (fence.kind === 'polygon' && Array.isArray(fence.polygon) && fence.polygon.length > 2) {
        const points = fence.polygon.map(([lon, lat]) => this.project(lat, lon, width, height));
        ctx.beginPath();
        points.forEach((point, index) => index === 0 ? ctx.moveTo(point.x, point.y) : ctx.lineTo(point.x, point.y));
        ctx.closePath();
        ctx.fillStyle = this.hexToRgba(fence.color, alpha);
        ctx.fill();
        ctx.strokeStyle = this.hexToRgba(fence.color, highlighted ? .9 : .55);
        ctx.lineWidth = highlighted ? 2 : 1.2;
        ctx.setLineDash([5, 5]);
        ctx.stroke();
        ctx.setLineDash([]);
        const center = points.reduce((acc, point) => ({ x: acc.x + point.x / points.length, y: acc.y + point.y / points.length }), { x: 0, y: 0 });
        if (highlighted) {
          this.fenceLabel(fence, center.x, center.y, highlighted);
        } else {
          ctx.beginPath();
          ctx.arc(center.x, center.y, 2.2, 0, Math.PI * 2);
          ctx.fillStyle = this.hexToRgba(fence.color, .55);
          ctx.fill();
        }
      } else {
        const center = this.project(fence.center_lat, fence.center_lon, width, height);
        const { minLon, maxLon, minLat, maxLat } = this.bounds;
        const rx = (fence.radius_m / 111320 / Math.cos(fence.center_lat * Math.PI / 180)) / (maxLon - minLon) * (width - 48);
        const ry = (fence.radius_m / 111320) / (maxLat - minLat) * (height - 48);
        ctx.beginPath();
        ctx.ellipse(center.x, center.y, Math.max(3, rx), Math.max(3, ry), 0, 0, Math.PI * 2);
        ctx.fillStyle = this.hexToRgba(fence.color, alpha);
        ctx.fill();
        ctx.strokeStyle = this.hexToRgba(fence.color, highlighted ? .9 : .55);
        ctx.lineWidth = highlighted ? 2 : 1.2;
        ctx.setLineDash([5, 5]);
        ctx.stroke();
        ctx.setLineDash([]);
      if (highlighted) {
        this.fenceLabel(fence, center.x, center.y - Math.max(7, ry) - 6, highlighted);
      } else {
        ctx.beginPath();
        ctx.arc(center.x, center.y, 2.2, 0, Math.PI * 2);
        ctx.fillStyle = this.hexToRgba(fence.color, .55);
        ctx.fill();
      }
      }
      ctx.restore();
    });
  }

  fenceLabel(fence, x, y, highlighted) {
    const ctx = this.ctx;
    ctx.font = `${highlighted ? '600 ' : ''}9px "Segoe UI", "Microsoft YaHei", sans-serif`;
    const text = fence.name;
    const widthValue = ctx.measureText(text).width + 12;
    ctx.fillStyle = highlighted ? 'rgba(9, 22, 34, .92)' : 'rgba(8, 19, 31, .68)';
    this.roundRect(x - widthValue / 2, y - 13, widthValue, 17, 4);
    ctx.fill();
    ctx.fillStyle = this.hexToRgba(fence.color, highlighted ? 1 : .75);
    ctx.textAlign = 'center';
    ctx.fillText(text, x, y - 1);
  }

  drawTrajectory(width, height) {
    const ctx = this.ctx;
    const points = this.trajectory.filter((point) => Number.isFinite(Number(point.lat)) && Number.isFinite(Number(point.lon)));
    if (points.length < 2) return;
    const projected = points.map((point) => this.project(point.lat, point.lon, width, height));
    ctx.save();
    ctx.lineJoin = 'round';
    ctx.lineCap = 'round';
    ctx.beginPath();
    projected.forEach((point, index) => index === 0 ? ctx.moveTo(point.x, point.y) : ctx.lineTo(point.x, point.y));
    ctx.strokeStyle = 'rgba(45, 212, 191, .14)';
    ctx.lineWidth = 7;
    ctx.stroke();
    ctx.strokeStyle = 'rgba(45, 212, 191, .82)';
    ctx.lineWidth = 1.8;
    ctx.stroke();
    const start = projected[0];
    const end = projected[projected.length - 1];
    ctx.beginPath();
    ctx.arc(start.x, start.y, 3.2, 0, Math.PI * 2);
    ctx.fillStyle = '#82e8c8';
    ctx.fill();
    ctx.beginPath();
    ctx.arc(end.x, end.y, 4 + this.pulse * 5, 0, Math.PI * 2);
    ctx.strokeStyle = `rgba(45, 212, 191, ${.6 * (1 - this.pulse)})`;
    ctx.lineWidth = 1.4;
    ctx.stroke();
    ctx.restore();
  }

  drawVehicles(width, height) {
    const ctx = this.ctx;
    this.screen.clear();
    this.vehicles.forEach((vehicle) => {
      const lat = Number(vehicle.lat ?? vehicle.state?.lat);
      const lon = Number(vehicle.lon ?? vehicle.state?.lon);
      if (!Number.isFinite(lat) || !Number.isFinite(lon)) return;
      const point = this.project(lat, lon, width, height);
      this.screen.set(vehicle.id, point);
      const status = vehicle.status || vehicle.state?.status || 'offline';
      const meta = STATUS[status] || STATUS.offline;
      const selected = vehicle.id === this.selectedId;
      const hovered = vehicle.id === this.hoverId;
      const color = meta.color;

      if (selected) {
        const radius = 12 + this.pulse * 12;
        ctx.beginPath();
        ctx.arc(point.x, point.y, radius, 0, Math.PI * 2);
        ctx.strokeStyle = `rgba(45, 212, 191, ${.6 * (1 - this.pulse)})`;
        ctx.lineWidth = 1.2;
        ctx.stroke();
      }
      ctx.save();
      ctx.translate(point.x, point.y);
      if (status === 'moving') {
        ctx.rotate((Number(vehicle.heading ?? vehicle.state?.heading ?? 0) * Math.PI) / 180);
        ctx.beginPath();
        ctx.moveTo(0, -7.5);
        ctx.lineTo(5.4, 5);
        ctx.lineTo(0, 2.6);
        ctx.lineTo(-5.4, 5);
        ctx.closePath();
        ctx.fillStyle = color;
        ctx.shadowColor = color;
        ctx.shadowBlur = selected || hovered ? 15 : 7;
        ctx.fill();
      } else {
        ctx.beginPath();
        ctx.arc(0, 0, selected ? 5.2 : 3.8, 0, Math.PI * 2);
        ctx.fillStyle = color;
        ctx.shadowColor = color;
        ctx.shadowBlur = selected || hovered ? 14 : 6;
        ctx.fill();
        ctx.beginPath();
        ctx.arc(0, 0, selected ? 8.5 : 6.5, 0, Math.PI * 2);
        ctx.strokeStyle = this.hexToRgba(color, .35);
        ctx.lineWidth = 1;
        ctx.stroke();
      }
      ctx.restore();

      if (selected || hovered) {
        const label = vehicle.plate_no || '';
        ctx.font = '600 9px "Segoe UI", "Microsoft YaHei", sans-serif';
        const textWidth = ctx.measureText(label).width + 12;
        const labelY = point.y - 22;
        ctx.fillStyle = 'rgba(5, 14, 24, .9)';
        this.roundRect(point.x - textWidth / 2, labelY - 12, textWidth, 17, 4);
        ctx.fill();
        ctx.strokeStyle = 'rgba(45, 212, 191, .28)';
        ctx.lineWidth = 1;
        this.roundRect(point.x - textWidth / 2, labelY - 12, textWidth, 17, 4);
        ctx.stroke();
        ctx.fillStyle = '#dffbf6';
        ctx.textAlign = 'center';
        ctx.fillText(label, point.x, labelY);
      }
    });
  }

  draw() {
    const rect = this.canvas.getBoundingClientRect();
    const width = Math.max(1, rect.width);
    const height = Math.max(1, rect.height);
    if (!width || !height) return;
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    if (this.canvas.width !== Math.round(width * dpr) || this.canvas.height !== Math.round(height * dpr)) {
      this.canvas.width = Math.round(width * dpr);
      this.canvas.height = Math.round(height * dpr);
    }
    const ctx = this.ctx;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, height);
    if (this.fitTrajectory) this.bounds = this.zoomForTrajectory();
    this.drawBackground(width, height);
    this.drawFences(width, height);
    this.drawTrajectory(width, height);
    this.drawVehicles(width, height);
  }

  handleMove(event) {
    const rect = this.canvas.getBoundingClientRect();
    const x = event.clientX - rect.left;
    const y = event.clientY - rect.top;
    let nearest = null;
    let distance = 16;
    this.screen.forEach((point, id) => {
      const dist = Math.hypot(point.x - x, point.y - y);
      if (dist < distance) { distance = dist; nearest = id; }
    });
    if (!nearest) { this.hoverId = null; this.tooltip.style.display = 'none'; return; }
    this.hoverId = nearest;
    const vehicle = this.vehicles.find((item) => item.id === nearest);
    if (!vehicle) return;
    const meta = STATUS[vehicle.status || vehicle.state?.status] || STATUS.offline;
    this.tooltip.innerHTML = `<strong>${escapeHtml(vehicle.plate_no)}</strong><span>${escapeHtml(vehicle.model || '')}</span><span><b>${Math.round(Number(vehicle.speed ?? vehicle.state?.speed ?? 0))} km/h</b> · ${meta.label} · SOC ${Math.round(Number(vehicle.soc ?? vehicle.state?.soc ?? 0))}%</span>`;
    this.tooltip.style.display = 'block';
    this.tooltip.style.left = `${x}px`;
    this.tooltip.style.top = `${y - 4}px`;
  }

  handleClick(event) {
    const rect = this.canvas.getBoundingClientRect();
    const x = event.clientX - rect.left;
    const y = event.clientY - rect.top;
    let nearest = null;
    let distance = 16;
    this.screen.forEach((point, id) => {
      const dist = Math.hypot(point.x - x, point.y - y);
      if (dist < distance) { distance = dist; nearest = id; }
    });
    if (nearest) this.onSelect(nearest);
  }

  roundRect(x, y, width, height, radius) {
    const ctx = this.ctx;
    ctx.beginPath();
    ctx.moveTo(x + radius, y);
    ctx.arcTo(x + width, y, x + width, y + height, radius);
    ctx.arcTo(x + width, y + height, x, y + height, radius);
    ctx.arcTo(x, y + height, x, y, radius);
    ctx.arcTo(x, y, x + width, y, radius);
    ctx.closePath();
  }

  hexToRgba(hex, alpha) {
    const value = String(hex || '#22d3ee').replace('#', '');
    const full = value.length === 3 ? value.split('').map((char) => char + char).join('') : value;
    const number = parseInt(full, 16);
    const r = (number >> 16) & 255, g = (number >> 8) & 255, b = number & 255;
    return `rgba(${r}, ${g}, ${b}, ${alpha})`;
  }
}

/* ---------------------------------------------------------------- chrome UI */

function startClock() {
  const tick = () => {
    const now = new Date();
    const time = $('#clockTime');
    const date = $('#clockDate');
    if (time) time.textContent = now.toLocaleTimeString('zh-CN', { hour12: false, hourCycle: 'h23' });
    if (date) date.textContent = new Intl.DateTimeFormat('zh-CN', { month: 'long', day: 'numeric', weekday: 'short' }).format(now);
  };
  tick();
  setInterval(tick, 1000);
}

function closeSidebar() {
  $('#sidebar')?.classList.remove('open');
  $('#sidebarOverlay')?.classList.remove('open');
}

async function handleAction(element, event) {
  const action = element.dataset.action;
  const id = element.dataset.id;
  switch (action) {
    case 'close-modal': closeModal(); break;
    case 'refresh': refreshCurrent(); break;
    case 'add-vehicle': openAddVehicleModal(); break;
    case 'add-geofence': openAddGeofenceModal(); break;
    case 'vehicle-detail': openVehicleModal(id); break;
    case 'dispatch-command': openCommandModal(id); break;
    case 'locate-vehicle':
      state.selectedVehicleId = id;
      closeModal();
      navigate('live', { vehicleId: id });
      break;
    case 'select-live': selectLiveVehicle(id); break;
    case 'quick-command': sendCommand(id, element.dataset.command); break;
    case 'ack-alert': acknowledgeAlert(id); break;
    case 'ack-all': acknowledgeAll(); break;
    case 'toggle-fence': {
      const fence = state.geofences.find((item) => item.id === id);
      if (fence) toggleFence(id, fence.active);
      break;
    }
    case 'delete-fence': {
      const ok = await confirmDialog({ title: '删除电子围栏？', message: '删除后围栏配置与进出检测状态将一并清除，历史告警仍会保留。', confirmText: '删除', danger: true });
      if (!ok) return;
      try { await api(`/geofences/${encodeURIComponent(id)}`, { method: 'DELETE' }); toast('围栏已删除'); await loadGeofences(); }
      catch (error) { toast('删除失败', error.message, 'error'); }
      break;
    }
    case 'bind-terminal': openBindTerminalModal(id); break;
    case 'add-terminal': openAddTerminalModal(); break;
    case 'view-trip': openTripModal(id); break;
    case 'page': {
      const page = Number(element.dataset.page);
      if (element.dataset.scope === 'vehicles') { state.vehiclePage = page; await loadVehicles(); }
      if (element.dataset.scope === 'alerts') { state.alertPage = page; await loadAlerts(); }
      if (element.dataset.scope === 'trips') { state.tripPage = page; await loadTrips(); }
      break;
    }
    case 'send-command': {
      if (!state.commandVehicleId) return;
      await sendCommand(state.commandVehicleId, state.commandType);
      break;
    }
    case 'alert-focus': {
      state.alertFilter = 'active';
      await navigate('alerts');
      break;
    }
    case 'logout': logout(); break;
    default: break;
  }
}

/* ------------------------------------------------------------------- boot */

document.addEventListener('DOMContentLoaded', () => {
  startClock();
  $('#loginForm').addEventListener('submit', login);
  $('#togglePassword').addEventListener('click', () => {
    const input = $('#password');
    const visible = input.type === 'text';
    input.type = visible ? 'password' : 'text';
    $('#togglePassword').textContent = visible ? '显示' : '隐藏';
  });
  $$('.demo-chip').forEach((chip) => chip.addEventListener('click', () => {
    const map = {
      admin: ['admin', 'admin123'],
      operator: ['operator', 'operator123'],
      viewer: ['viewer', 'viewer123']
    };
    const [username, password] = map[chip.dataset.demo] || [];
    $('#username').value = username || '';
    $('#password').value = password || '';
    setLoginError('');
    chip.blur();
  }));
  $$('.nav-item').forEach((item) => item.addEventListener('click', () => navigate(item.dataset.view)));
  $('#menuBtn')?.addEventListener('click', () => {
    $('#sidebar').classList.add('open');
    $('#sidebarOverlay').classList.add('open');
  });
  $('#sidebarOverlay')?.addEventListener('click', closeSidebar);
  $('#userMenuBtn')?.addEventListener('click', (event) => {
    event.stopPropagation();
    $('#userMenu')?.classList.toggle('hidden');
  });
  document.addEventListener('click', (event) => {
    if (!event.target.closest('.user-menu-wrap')) $('#userMenu')?.classList.add('hidden');
  });
  $('#logoutBtn')?.addEventListener('click', () => logout());
  $('#globalSearch')?.addEventListener('keydown', (event) => {
    if (event.key !== 'Enter') return;
    const value = event.target.value.trim();
    state.vehicleSearch = value;
    state.vehiclePage = 1;
    navigate('vehicles');
  });
  document.addEventListener('keydown', (event) => {
    if (event.key === 'Escape') {
      if ($('#modalRoot').classList.contains('open')) closeModal();
      closeSidebar();
      $('#userMenu')?.classList.add('hidden');
    }
  });
  document.addEventListener('click', (event) => {
    const nav = event.target.closest('[data-nav]');
    if (nav) { navigate(nav.dataset.nav); return; }
    const action = event.target.closest('[data-action]');
    if (!action) return;
    if (action.closest('tr[data-action]') && action !== action.closest('tr[data-action]')) {
      event.stopPropagation();
    }
    handleAction(action, event);
  });
  if (state.token) {
    enterApp().catch(() => {
      state.token = '';
      localStorage.removeItem('tsp_token');
      showLogin();
      setLoginError('登录状态已失效，请重新登录');
    });
  } else {
    showLogin();
  }
});
