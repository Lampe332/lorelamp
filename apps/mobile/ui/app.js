// Start page of the Android app: find Lorelamp on a PC, then hand the WebView over to it.
// Everything that touches the network or checks an address lives in the Rust part.
'use strict';

const invoke = window.__TAURI__.core.invoke;
const $ = (id) => document.getElementById(id);

// Error codes from the Rust part (link.rs `Fail`) and of the camera, in the phone's language (i18n.js).
const ERRORS = new Set(['not_a_link', 'not_http', 'not_local', 'no_token', 'bad_address', 'bad_code', 'unreachable',
  'not_lorelamp', 'token_rejected', 'wrong_code', 'too_many', 'lan_off', 'storage', 'camera']);
const sentence = (code) => t(ERRORS.has(code) ? `err.${code}` : 'err.other');

/** How deep a view lies, so going on slides one way and going back the other. */
const DEPTH = { load: 0, start: 1, down: 1, offer: 1, scan: 2, code: 2 };

function show(view) {
  const body = document.body;
  const from = body.dataset.view;
  if (from === view) return;
  body.dataset.dir = DEPTH[view] < DEPTH[from] ? 'back' : 'on';
  body.dataset.view = view;
  if (view !== 'down') stopWatching();
}

/** A message in its box (the box carries the icon); no text hides the box. */
function say(el, text) {
  el.textContent = text || '';
  el.parentElement.hidden = !text;
}

/** The PC's interface replaces this page, so "back" on its first page leaves the app. */
function openPc(url) {
  location.replace(url);
}

function showStart(message) {
  say($('start-msg'), message);
  show('start');
}

function showLoad(text) {
  $('load-text').textContent = text;
  show('load');
}

async function openSaved() {
  showLoad(t('load.connectingPc'));
  try {
    openPc(await invoke('open_saved'));
  } catch (code) {
    if (code === 'unreachable') showDown();
    else showStart(sentence(code === 'none_saved' ? 'token_rejected' : code));
  }
}

async function connect(raw) {
  showLoad(t('load.connecting'));
  try {
    openPc(await invoke('connect', { raw }));
  } catch (code) {
    showStart(sentence(code));
  }
}

// --- A link from outside -----------------------------------------------------
// Any web page or app on the phone can open lorelamp://connect?u=…, so it only ever becomes a
// question: which PC, and which remembered one a yes replaces. Nothing is stored before the yes.

function showOffer(offer, saved) {
  $('offer-target').textContent = offer.target;
  const replace = $('offer-replace');
  replace.textContent = offer.replaces ? t('offer.replace', { pc: offer.replaces }) : '';
  replace.hidden = !offer.replaces;
  $('offer-no').dataset.saved = saved ? '1' : '';
  show('offer');
}

async function acceptOffer() {
  showLoad(t('load.connecting'));
  try {
    openPc(await invoke('accept_link'));
  } catch (code) {
    showStart(sentence(code));
  }
}

async function declineOffer() {
  const saved = $('offer-no').dataset.saved === '1';
  try {
    await invoke('decline_link');
  } catch {
    // nothing was waiting
  }
  if (saved) openSaved();
  else showStart('');
}

// --- PC not reachable --------------------------------------------------------
// The PC is often just waking up: while this view is on screen the app asks again every few
// seconds and opens the PC by itself as soon as it answers.

const WATCH_MS = 4000;
let watching = 0; // counts the runs, so an old loop stops when a new one starts

function stopWatching() {
  watching += 1;
}

async function watch() {
  const run = (watching += 1);
  const alive = () => run === watching && document.body.dataset.view === 'down' && !document.hidden;
  while (alive()) {
    await new Promise((done) => setTimeout(done, WATCH_MS));
    if (!alive()) return;
    try {
      const url = await invoke('open_saved');
      if (alive()) openPc(url);
      return;
    } catch (code) {
      if (code !== 'unreachable') {
        // The PC answers but no longer knows this phone: nothing to wait for.
        if (alive()) showStart(sentence(code === 'none_saved' ? 'token_rejected' : code));
        return;
      }
    }
  }
}

function showDown() {
  show('down');
  watch();
}

document.addEventListener('visibilitychange', () => {
  if (document.hidden) stopWatching();
  else if (document.body.dataset.view === 'down') watch();
});

// --- QR code ---------------------------------------------------------------

async function scan() {
  say($('start-msg'), '');
  $('camera-settings').hidden = true;
  let granted = false;
  try {
    let state = await invoke('plugin:barcode-scanner|check_permissions');
    if (state.camera !== 'granted') state = await invoke('plugin:barcode-scanner|request_permissions');
    granted = state.camera === 'granted';
  } catch {
    granted = false;
  }
  if (!granted) {
    $('camera-settings').hidden = false;
    showStart(sentence('camera'));
    return;
  }
  show('scan');
  let content = null;
  try {
    const hit = await invoke('plugin:barcode-scanner|scan', { windowed: true, formats: ['QR_CODE'] });
    content = hit && hit.content;
  } catch {
    content = null; // cancelled
  }
  if (content) connect(content);
  else if (document.body.dataset.view === 'scan') show('start');
}

/** Ends the scan and goes to `next` (the start page, or the code entry). */
async function leaveScan(next) {
  next();
  try {
    await invoke('plugin:barcode-scanner|cancel');
  } catch {
    // nothing was running
  }
}

// --- Code ------------------------------------------------------------------

const useIcon = (id, extra) => {
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('class', extra ? `ic ${extra}` : 'ic');
  svg.setAttribute('aria-hidden', 'true');
  const use = document.createElementNS('http://www.w3.org/2000/svg', 'use');
  use.setAttribute('href', `#${id}`);
  svg.append(use);
  return svg;
};

function pcRow(pc, checked) {
  const label = document.createElement('label');
  label.className = 'pick';
  const radio = document.createElement('input');
  radio.type = 'radio';
  radio.name = 'pc';
  radio.value = pc.addr;
  radio.checked = checked;
  const text = document.createElement('span');
  text.textContent = pc.name;
  const addr = document.createElement('small');
  addr.textContent = pc.addr;
  text.append(addr);
  label.append(radio, useIcon('i-pc'), text, useIcon('i-check', 'tick'));
  return label;
}

function syncManual() {
  $('manual').hidden = !$('manual-radio').checked;
}

/** The six digits in the field, whatever was typed or pasted around them ("123 456", "Code: 123456"). */
const digits = (text) => text.replace(/\D/g, '').slice(0, 6);

/** Draws the field's digits into the six cells and marks the one that is typed next. */
function drawCode() {
  const field = $('code');
  const value = field.value;
  const active = document.activeElement === field;
  const cells = $('otp').querySelectorAll('.cells i');
  cells.forEach((cell, i) => {
    cell.textContent = value[i] || '';
    cell.classList.toggle('filled', i < value.length);
    cell.classList.toggle('next', active && i === Math.min(value.length, 5));
  });
}

function setCode(text) {
  const field = $('code');
  field.value = digits(text);
  $('otp').classList.remove('wrong');
  drawCode();
}

/** The address that is chosen right now: a found PC's, or the typed one. */
function pickedAddr() {
  const picked = document.querySelector('input[name="pc"]:checked');
  return picked && picked.value ? picked.value : $('addr').value.trim();
}

function onCodeInput() {
  say($('code-msg'), '');
  setCode($('code').value);
  // All six digits and a PC to send them to: no need to press the button as well.
  if ($('code').value.length === 6 && pickedAddr() && !$('pair').disabled) $('code-form').requestSubmit();
}

let searches = 0;

async function search() {
  const run = (searches += 1);
  const state = $('search-state');
  state.hidden = false;
  state.classList.add('busy');
  state.textContent = t('code.searching');
  $('pcs').replaceChildren();
  $('manual-pick').hidden = true;
  $('manual').hidden = true;
  $('search-again').hidden = true;
  let pcs = [];
  try {
    pcs = await invoke('discover');
  } catch {
    pcs = [];
  }
  if (run !== searches) return; // a newer search took over
  $('pcs').replaceChildren(...pcs.map((pc, i) => pcRow(pc, i === 0)));
  $('manual-radio').checked = pcs.length === 0;
  $('manual-pick').hidden = pcs.length === 0;
  $('search-again').hidden = false;
  state.classList.remove('busy');
  if (pcs.length === 0) state.textContent = t('code.noneFound');
  else state.hidden = true;
  syncManual();
  // No PC answered: the address comes first, so the keyboard changes to it (unless a code is being typed).
  if (pcs.length === 0 && document.body.dataset.view === 'code' && !$('code').value) $('addr').focus({ preventScroll: true });
  // A code typed while the search ran goes out as soon as there is a PC for it.
  if (document.body.dataset.view === 'code' && $('code').value.length === 6 && pickedAddr()) $('code-form').requestSubmit();
}

function openCode() {
  say($('code-msg'), '');
  setCode('');
  show('code');
  search();
  // Straight to the digits: the number pad opens with the page.
  $('code').focus({ preventScroll: true });
}

async function pair(event) {
  event.preventDefault();
  const button = $('pair');
  if (button.disabled) return;
  say($('code-msg'), '');
  button.disabled = true;
  button.textContent = t('load.connecting');
  try {
    openPc(await invoke('pair', { addr: pickedAddr(), code: $('code').value }));
  } catch (code) {
    say($('code-msg'), sentence(code));
    button.disabled = false;
    button.textContent = t('code.pair');
    if (code === 'wrong_code' || code === 'bad_code') {
      // A wrong code is typed again from the start; the cells shake once.
      $('code').value = '';
      drawCode();
      $('otp').classList.add('wrong');
      $('code').focus({ preventScroll: true });
    }
  }
}

// --- Start -----------------------------------------------------------------

$('scan').addEventListener('click', scan);
$('scan-cancel').addEventListener('click', () => leaveScan(() => show('start')));
$('scan-to-code').addEventListener('click', () => leaveScan(openCode));
$('to-code').addEventListener('click', openCode);
$('code-back').addEventListener('click', () => showStart(''));
$('code-form').addEventListener('submit', pair);
$('code-form').addEventListener('change', syncManual);
$('code').addEventListener('input', onCodeInput);
$('code').addEventListener('focus', drawCode);
$('code').addEventListener('blur', drawCode);
$('otp').addEventListener('animationend', () => $('otp').classList.remove('wrong'));
$('search-again').addEventListener('click', search);
$('retry').addEventListener('click', openSaved);
$('reconnect').addEventListener('click', () => showStart(''));
$('back-to-pc').addEventListener('click', openSaved);
$('camera-settings').addEventListener('click', () => invoke('plugin:barcode-scanner|open_app_settings').catch(() => {}));
$('offer-yes').addEventListener('click', acceptOffer);
$('offer-no').addEventListener('click', declineOffer);

(async function boot() {
  let start = { offer: null, offer_fail: null, saved: false };
  try {
    start = await invoke('startup');
  } catch {
    // shows the start page
  }
  const rescan = new URLSearchParams(location.search).has('rescan');
  $('back-to-pc').hidden = !(rescan && start.saved);
  if (start.offer) showOffer(start.offer, start.saved);
  else if (start.offer_fail) showStart(sentence(start.offer_fail));
  else if (start.saved && !rescan) openSaved();
  else showStart('');
})();
