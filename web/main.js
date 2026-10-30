// M0 Demo：生成世界 → 駕駛。只保留驗證地圖生成與道路所需的最少介面。
import init, { WorldLoader, create_game } from "./pkg/game.js";
import { STRINGS, detectLang } from "./i18n.js";

const T = STRINGS[detectLang()];
const $ = (id) => document.getElementById(id);
document.querySelectorAll("[data-t]").forEach((el) => { const v = T[el.dataset.t]; if (typeof v === "string") el.textContent = v; });
document.querySelectorAll(".lang-select").forEach((el) => el.closest("label").classList.add("hidden"));
$("seed").value = "20261030";
$("seed-official").onclick = () => ($("seed").value = "20261030");
$("seed-random").onclick = () => ($("seed").value = String(Math.floor(Math.random() * 1e9)));
const touch = matchMedia("(pointer: coarse)").matches;
if (touch) document.body.classList.add("touch");

let game = null;
const canvas = $("gfx");
function fit() {
  const dpr = Math.min(window.devicePixelRatio || 1, 2);
  canvas.width = Math.floor(canvas.clientWidth * dpr);
  canvas.height = Math.floor(canvas.clientHeight * dpr);
  if (game) game.resize(canvas.width, canvas.height);
}
addEventListener("resize", fit);

$("start").onclick = async () => {
  if (!navigator.gpu) { $("err").textContent = T.noWebGPU; return; }
  $("menu").classList.add("hidden");
  $("loading").classList.remove("hidden");
  try {
    await init();
    const loader = new WorldLoader($("seed").value, $("size").value);
    let p = 0;
    while (p < 1) {
      $("stage").textContent = T.stages[loader.stage()] || loader.stage();
      await new Promise((r) => requestAnimationFrame(r));
      p = loader.step();
      $("bar-fill").style.width = `${Math.round(p * 100)}%`;
    }
    fit();
    game = await create_game(canvas, loader, $("quality").value);
    $("loading").classList.add("hidden");
    $("hud").classList.remove("hidden");
    if (touch) $("touch").classList.remove("hidden");
    setupInput();
    drawMapBase();
    requestAnimationFrame(loop);
  } catch (e) {
    console.error(e);
    $("loading").classList.add("hidden");
    $("menu").classList.remove("hidden");
    $("err").textContent = `${T.error}: ${e}`;
  }
};

function setupInput() {
  addEventListener("keydown", (e) => { game.key(e.code, true); if (e.code.startsWith("Arrow") || e.code === "Space") e.preventDefault(); });
  addEventListener("keyup", (e) => game.key(e.code, false));
  addEventListener("blur", () => game.clear_keys());
  document.querySelectorAll("[data-key]").forEach((b) => {
    const k = b.dataset.key;
    b.addEventListener("pointerdown", (e) => { e.preventDefault(); b.classList.add("active"); game.key(k, true); });
    const up = () => { b.classList.remove("active"); game.key(k, false); };
    b.addEventListener("pointerup", up); b.addEventListener("pointercancel", up); b.addEventListener("pointerleave", up);
  });
  let drag = null;
  canvas.addEventListener("pointerdown", (e) => (drag = [e.clientX, e.clientY]));
  addEventListener("pointermove", (e) => { if (drag) { game.orbit(e.clientX - drag[0], e.clientY - drag[1], true); drag = [e.clientX, e.clientY]; } });
  addEventListener("pointerup", () => { drag = null; game.orbit(0, 0, false); });
  // 設定面板在 Demo 中只保留時間與除錯資訊
  $("btn-settings").onclick = () => $("settings").classList.toggle("hidden");
  $("close-settings").onclick = () => $("settings").classList.add("hidden");
  $("time").oninput = (e) => game.set_hours(parseFloat(e.target.value));
  $("time-speed").onchange = (e) => game.set_time_speed(parseFloat(e.target.value));
  $("trans").onchange = (e) => game.set_transmission(e.target.value === "auto");
  $("fuel").onchange = (e) => game.set_fuel_enabled(e.target.value === "on");
  $("assists").onchange = (e) => game.set_assists(e.target.value === "on", e.target.value === "on");
  $("quality-ingame").onchange = (e) => game.set_quality(e.target.value);
  $("new-world").onclick = () => location.reload();
  $("copy-debug").onclick = async () => {
    const txt = `${navigator.userAgent}\n${game.debug_info()}`;
    try { await navigator.clipboard.writeText(txt); $("copy-debug").textContent = T.copied; } catch (_) { prompt("", txt); }
  };
}

// 小地圖：世界俯視圖 + 車輛位置
const mapImg = document.createElement("canvas");
function drawMapBase() {
  const rgba = game.map_rgba();
  const n = Math.round(Math.sqrt(rgba.length / 4));
  mapImg.width = mapImg.height = n;
  mapImg.getContext("2d").putImageData(new ImageData(new Uint8ClampedArray(rgba), n, n), 0, 0);
}
function drawMinimap(h) {
  const c = $("minimap"), ctx = c.getContext("2d");
  const s = game.world_size(), n = mapImg.width, view = 1600; // 小地圖顯示範圍（米）
  const px = (h.x / s) * n, pz = (h.z / s) * n, span = (view / s) * n;
  ctx.save();
  ctx.clearRect(0, 0, c.width, c.height);
  ctx.translate(c.width / 2, c.height / 2);
  ctx.rotate(Math.PI + h.yaw); // 車頭朝上
  ctx.drawImage(mapImg, px - span / 2, pz - span / 2, span, span, -c.width / 2, -c.height / 2, c.width, c.height);
  ctx.restore();
  ctx.fillStyle = "#ffb347";
  ctx.beginPath(); ctx.moveTo(80, 70); ctx.lineTo(86, 88); ctx.lineTo(74, 88); ctx.fill();
}

let frames = 0;
window.__game = () => game;
function loop(t) {
  if (window.__pause) { requestAnimationFrame(loop); return; }
  game.frame(t);
  if ((frames++ & 3) === 0) {
    const h = JSON.parse(game.hud());
    $("speed").textContent = Math.round(h.speed);
    $("gear").textContent = (h.auto ? "A" : "M") + h.gear;
    $("rpm").style.width = `${Math.min(100, (h.rpm / h.redline) * 100)}%`;
    $("fuelbar").style.width = `${h.fuel * 100}%`;
    $("notice").textContent = h.refuel ? T.refuel : h.fuelOn && h.fuel < 0.12 ? T.lowFuel : "";
    $("fps").textContent = `${game.fps().toFixed(0)} fps`;
    const hh = Math.floor(h.hours), mm = Math.floor((h.hours - hh) * 60);
    $("time-val").textContent = `${String(hh).padStart(2, "0")}:${String(mm).padStart(2, "0")}`;
    drawMinimap(h);
  }
  requestAnimationFrame(loop);
}
