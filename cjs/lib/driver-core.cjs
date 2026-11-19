const fs = require('fs');
const os = require('os');
const path = require('path');
const http = require('http');
const crypto = require('crypto');
const { spawn } = require('child_process');

const VERSION = '0.2.0';
const BASE = process.env.DS_WEB_BASE || path.join(os.homedir(), '.dsweb');

function sleep(ms) { return new Promise((r) => setTimeout(r, ms)); }

function getJson(url) {
  return new Promise((resolve, reject) => {
    http.get(url, (res) => {
      let data = '';
      res.on('data', (c) => { data += c; });
      res.on('end', () => {
        try { resolve(JSON.parse(data)); } catch (e) { reject(e); }
      });
    }).on('error', reject);
  });
}

function encodeFrame(payload, opcode = 0x1) {
  const len = payload.length;
  const mask = crypto.randomBytes(4);
  let header;
  if (len < 126) {
    header = Buffer.alloc(2);
    header[1] = 0x80 | len;
  } else if (len < 65536) {
    header = Buffer.alloc(4);
    header[1] = 0x80 | 126;
    header.writeUInt16BE(len, 2);
  } else {
    header = Buffer.alloc(10);
    header[1] = 0x80 | 127;
    header.writeBigUInt64BE(BigInt(len), 2);
  }
  header[0] = 0x80 | opcode;
  const body = Buffer.from(payload);
  for (let i = 0; i < body.length; i++) body[i] ^= mask[i % 4];
  return Buffer.concat([header, mask, body]);
}

function makeWs(socket, head) {
  let buf = head && head.length ? Buffer.from(head) : Buffer.alloc(0);
  let frags = [];
  const onMessage = [];
  const onClose = [];

  function dispatch(full) {
    const text = full.toString('utf8');
    for (const fn of onMessage) fn(text);
  }

  function feed() {
    for (;;) {
      if (buf.length < 2) return;
      const b0 = buf[0];
      const b1 = buf[1];
      const fin = (b0 & 0x80) !== 0;
      const opcode = b0 & 0x0f;
      const masked = (b1 & 0x80) !== 0;
      let len = b1 & 0x7f;
      let off = 2;
      if (len === 126) {
        if (buf.length < 4) return;
        len = buf.readUInt16BE(2);
        off = 4;
      } else if (len === 127) {
        if (buf.length < 10) return;
        len = Number(buf.readBigUInt64BE(2));
        off = 10;
      }
      const maskLen = masked ? 4 : 0;
      if (buf.length < off + maskLen + len) return;
      let payload = buf.subarray(off + maskLen, off + maskLen + len);
      if (masked) {
        const mask = buf.subarray(off, off + 4);
        payload = Buffer.from(payload);
        for (let i = 0; i < payload.length; i++) payload[i] ^= mask[i % 4];
      }
      buf = buf.subarray(off + maskLen + len);
      if (opcode === 0x8) {
        try { socket.end(); } catch (e) { /* closed */ }
        for (const fn of onClose) fn();
        return;
      }
      if (opcode === 0x9) { socket.write(encodeFrame(payload, 0xA)); continue; }
      if (opcode === 0xA) continue;
      if (opcode === 0x0) {
        frags.push(payload);
        if (fin) { const full = Buffer.concat(frags); frags = []; dispatch(full); }
        continue;
      }
      frags = [payload];
      if (fin) { const full = Buffer.concat(frags); frags = []; dispatch(full); }
    }
  }

  socket.on('data', (chunk) => { buf = Buffer.concat([buf, chunk]); feed(); });
  socket.on('close', () => { for (const fn of onClose) fn(); });
  socket.on('error', () => { for (const fn of onClose) fn(); });

  return {
    send(text) { socket.write(encodeFrame(Buffer.from(text, 'utf8'))); },
    close() { try { socket.end(); } catch (e) { /* closed */ } },
    on(event, fn) { if (event === 'message') onMessage.push(fn); else if (event === 'close') onClose.push(fn); },
  };
}

function wsConnect(wsUrl, timeoutMs = 15000) {
  return new Promise((resolve, reject) => {
    const u = new URL(wsUrl);
    const req = http.request({
      host: u.hostname,
      port: u.port || 80,
      path: u.pathname + (u.search || ''),
      headers: {
        Connection: 'Upgrade',
        Upgrade: 'websocket',
        'Sec-WebSocket-Version': '13',
        'Sec-WebSocket-Key': crypto.randomBytes(16).toString('base64'),
      },
    });
    const timer = setTimeout(() => { req.destroy(); reject(new Error('ws connect timeout')); }, timeoutMs);
    req.on('upgrade', (res, socket, head) => { clearTimeout(timer); resolve(makeWs(socket, head)); });
    req.on('error', (err) => { clearTimeout(timer); reject(err); });
    req.end();
  });
}

class Cdp {
  constructor(ws) {
    this.ws = ws;
    this.seq = 0;
    this.pending = new Map();
    ws.on('message', (raw) => {
      let msg;
      try { msg = JSON.parse(raw); } catch (e) { return; }
      if (msg.id && this.pending.has(msg.id)) {
        const { resolve, reject } = this.pending.get(msg.id);
        this.pending.delete(msg.id);
        if (msg.error) reject(new Error(msg.error.message || 'cdp error'));
        else resolve(msg.result);
      }
    });
    ws.on('close', () => {
      for (const { reject } of this.pending.values()) reject(new Error('cdp closed'));
      this.pending.clear();
    });
  }

  call(method, params, sessionId) {
    const id = ++this.seq;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      const msg = { id, method, params: params || {} };
      if (sessionId) msg.sessionId = sessionId;
      this.ws.send(JSON.stringify(msg));
    });
  }
}

function findChrome() {
  const env = process.env.DS_WEB_CHROME || process.env.DSWEB_CHROME || '';
  if (env && fs.existsSync(env)) return env;
  const candidates = [
    'C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe',
    'C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe',
    'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe',
    'C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe',
  ];
  if (process.env.LOCALAPPDATA) {
    candidates.push(path.join(process.env.LOCALAPPDATA, 'Google', 'Chrome', 'Application', 'chrome.exe'));
    candidates.push(path.join(process.env.LOCALAPPDATA, 'Microsoft', 'Edge', 'Application', 'msedge.exe'));
  }
  candidates.push('/usr/bin/google-chrome', '/usr/bin/chromium', '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome');
  for (const p of candidates) { if (p && fs.existsSync(p)) return p; }
  throw new Error('Chrome/Edge not found; set DS_WEB_CHROME');
}

function createRuntime(site) {
  const log = (...a) => console.error('[' + site.id + ']', ...a);
  const logErr = (...a) => console.error('[' + site.id + '][err]', ...a);
  let browser = null;
  const streams = new Map();
  let streamSeq = 0;
  const handlers = {};

  function send(obj) {
    try { process.stdout.write(JSON.stringify(obj) + '\n'); } catch (e) { logErr('stdout write failed', e.message); }
  }
  function respond(id, ok, result, error) {
    if (ok) send({ id, ok: true, result });
    else send({ id, ok: false, error: String((error && error.message) || error) });
  }
  function emit(name, payload) { send({ event: name, ...(payload || {}) }); }

  async function attach(profile) {
    const base = process.env.DS_WEB_CDP.replace(/\/+$/, '');
    const version = await getJson(base + '/json/version');
    const pages = await getJson(base + '/json/list');
    const page = pages.find((t) => t.type === 'page' && String(t.url || '').includes(site.hostMatch))
      || pages.find((t) => t.type === 'page');
    if (!page) throw new Error('no attachable page at ' + base);
    const ws = await wsConnect(version.webSocketDebuggerUrl);
    const cdp = new Cdp(ws);
    const attached = await cdp.call('Target.attachToTarget', { targetId: page.id, flatten: true });
    const sessionId = attached.sessionId;
    await cdp.call('Page.enable', {}, sessionId);
    await cdp.call('Runtime.enable', {}, sessionId);
    browser = { child: null, cdp, sessionId, profile, headless: false, attached: true };
    log('attached to', page.url);
  }

  async function launch(profile, headless) {
    if (process.env.DS_WEB_CDP) return attach(profile);
    const dir = path.join(BASE, 'profiles', site.id + '-' + String(profile || 'default'));
    fs.mkdirSync(dir, { recursive: true });
    for (const f of ['DevToolsActivePort', 'SingletonLock', 'SingletonCookie', 'SingletonSocket']) {
      try { fs.rmSync(path.join(dir, f), { force: true, recursive: true }); } catch (e) { /* ignore */ }
    }
    const exe = findChrome();
    const args = [
      `--user-data-dir=${dir}`,
      '--remote-debugging-port=0',
      '--no-first-run',
      '--no-default-browser-check',
      '--disable-extensions',
      '--disable-blink-features=AutomationControlled',
      '--disable-gpu',
      '--no-sandbox',
      '--window-size=1280,900',
      'about:blank',
    ];
    if (process.env.DS_WEB_PROXY) args.unshift('--proxy-server=' + process.env.DS_WEB_PROXY);
    if (headless) args.unshift('--headless=new');
    log('launching', exe, 'headless=' + !!headless, 'profile=' + profile);
    const child = spawn(exe, args, { stdio: 'ignore' });
    child.on('exit', (code) => { log('browser exited', code); if (browser && browser.child === child) browser = null; });

    const portFile = path.join(dir, 'DevToolsActivePort');
    let port = 0;
    for (let i = 0; i < 125 && !port; i++) {
      try {
        if (fs.existsSync(portFile)) {
          const lines = fs.readFileSync(portFile, 'utf8').split(/\r?\n/);
          port = parseInt(lines[0], 10) || 0;
        }
      } catch (e) { /* retry */ }
      if (!port) await sleep(200);
    }
    if (!port) { try { child.kill(); } catch (e) { /* ignore */ } throw new Error('browser did not expose DevTools port'); }

    const version = await getJson(`http://127.0.0.1:${port}/json/version`);
    const ws = await wsConnect(version.webSocketDebuggerUrl);
    const cdp = new Cdp(ws);
    const target = await cdp.call('Target.createTarget', { url: 'about:blank' });
    const attached = await cdp.call('Target.attachToTarget', { targetId: target.targetId, flatten: true });
    const sessionId = attached.sessionId;
    await cdp.call('Page.enable', {}, sessionId);
    await cdp.call('Runtime.enable', {}, sessionId);
    browser = { child, cdp, sessionId, profile, headless: !!headless };
    log('cdp connected', version.Browser || '');
  }

  async function closeBrowser() {
    if (!browser) return;
    if (browser.attached) {
      try { browser.cdp.ws.close(); } catch (e) { /* ignore */ }
      browser = null;
      return;
    }
    const child = browser.child;
    try { await browser.cdp.call('Browser.close'); } catch (e) { /* ignore */ }
    await sleep(300);
    try { child.kill(); } catch (e) { /* ignore */ }
    browser = null;
  }

  async function ensureBrowser(profile, headless) {
    if (browser && browser.attached) return;
    if (browser && browser.profile === profile && browser.headless === !!headless) return;
    if (browser) await closeBrowser();
    await launch(profile, headless);
  }

  async function evalJs(expression) {
    if (!browser) throw new Error('browser not running');
    const r = await browser.cdp.call('Runtime.evaluate', {
      expression,
      returnByValue: true,
      awaitPromise: true,
    }, browser.sessionId);
    if (r.exceptionDetails) throw new Error(r.exceptionDetails.text || 'evaluate failed');
    return r.result ? r.result.value : undefined;
  }

  async function navigate(url) {
    await browser.cdp.call('Page.navigate', { url }, browser.sessionId);
    const start = Date.now();
    while (Date.now() - start < 30000) {
      try { if ((await evalJs('document.readyState')) === 'complete') return; } catch (e) { /* loading */ }
      await sleep(300);
    }
  }

  async function ensurePage() {
    let href = '';
    try { href = await evalJs('location.href'); } catch (e) { /* blank page */ }
    if (!href.startsWith('http') || !href.includes(site.hostMatch)) await navigate(site.url);
  }

  const E = site.exprs;

  async function settleModal() {
    if (!E.modal || !E.dismiss) return false;
    const visible = await evalJs(E.modal).catch(() => false);
    if (!visible) return false;
    await evalJs(E.dismiss).catch(() => false);
    await sleep(600);
    return true;
  }

  async function waitInput(timeoutMs) {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      const st = await evalJs(E.state);
      if (st.needsLogin) throw new Error(site.loginError || 'login required: open /login and sign in');
      if (await settleModal()) continue;
      if (await evalJs(E.input)) return st;
      await sleep(500);
    }
    throw new Error(site.readyError || 'chat page not ready');
  }

  async function prepare() {
    const ctx = { evalJs, sleep, log, site, settleModal, navigate };
    if (site.prepare) return site.prepare(ctx);
    return waitInput(60000);
  }

  async function resetChat() {
    if (E.newChat) {
      const clicked = await evalJs(E.newChat).catch(() => false);
      if (clicked) { await sleep(1200); return; }
    }
    await navigate(site.url);
    await sleep(1200);
  }

  async function pressEnter() {
    if (!browser) return;
    const base = { key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13, nativeVirtualKeyCode: 13 };
    for (const type of ['keyDown', 'char', 'keyUp']) {
      const ev = { ...base, type };
      if (type === 'char') ev.text = '\r';
      try { await browser.cdp.call('Input.dispatchKeyEvent', ev, browser.sessionId); } catch (e) { /* ignore */ }
    }
  }

  async function sendQuestion(question) {
    const before = await evalJs(E.userCount);
    log('send attempt baseline userCount=' + before);
    for (let i = 0; i < 3; i++) {
      await settleModal();
      let filled = false;
      if (site.cdpInput && E.focus) {
        const rect = E.rect ? await evalJs(E.rect).catch(() => null) : null;
        if (rect && rect.x && rect.y) {
          await browser.cdp.call('Input.dispatchMouseEvent', { type: 'mousePressed', x: rect.x, y: rect.y, button: 'left', clickCount: 1 }, browser.sessionId);
          await browser.cdp.call('Input.dispatchMouseEvent', { type: 'mouseReleased', x: rect.x, y: rect.y, button: 'left', clickCount: 1 }, browser.sessionId);
          await sleep(200);
        }
        filled = await evalJs(E.focus);
        await sleep(150);
        try {
          await browser.cdp.call('Input.insertText', { text: question }, browser.sessionId);
          filled = true;
        } catch (e) { filled = false; }
      }
      if (!filled) filled = await evalJs(E.fill(question));
      await sleep(300);
      const sent = await evalJs(E.send);
      await sleep(1000);
      let now = await evalJs(E.userCount);
      if (now <= before) {
        await pressEnter();
        await sleep(900);
        now = await evalJs(E.userCount);
      }
      if (await settleModal()) { await sleep(700); continue; }
      log('send attempt ' + (i + 1) + ' filled=' + filled + ' clicked=' + sent + ' userCount=' + now);
      if (now > before) return;
      if (E.sent) {
        for (let k = 0; k < 10; k++) {
          if (await evalJs(E.sent).catch(() => false)) {
            log('send attempt ' + (i + 1) + ' confirmed via sentinel');
            return;
          }
          await sleep(500);
        }
      }
      await sleep(500);
    }
    throw new Error('failed to send message');
  }

  function cleanText(text) {
    return String(text || '')
      .replace(/<think>[\s\S]*?<\/think>\n?/gi, '')
      .replace(/^Thinking[^\n]*\n?/gim, '')
      .replace(/\n{3,}/g, '\n\n')
      .trim();
  }

  function parseToolCalls(text, tools) {
    if (!Array.isArray(tools) || !tools.length) return [];
    const fenced = String(text).match(/```tool_call\s*([\s\S]*?)```/i);
    const bare = String(text).match(/^tool_call\s*\n([\s\S]+)$/i);
    const body = fenced ? fenced[1] : (bare ? bare[1] : '');
    if (!body.trim()) return [];
    try {
      const parsed = JSON.parse(body.trim());
      const name = parsed.name || parsed.tool || parsed.function;
      if (!name) return [];
      return [{ type: 'tool_call', name, args: parsed.args || parsed.arguments || {} }];
    } catch (e) {
      return [];
    }
  }

  handlers.ping = async () => ({ ok: true, version: VERSION, site: site.id });
  handlers.health = async () => ({ ok: true });
  handlers.models = async () => ({ site: site.id, models: site.models || [] });

  handlers.status = async () => {
    if (!browser) return { browser: { running: false }, site: site.id };
    let url = '';
    try { url = (await evalJs(E.state)).url; } catch (e) { /* ignore */ }
    return {
      browser: { running: true, profile: browser.profile, pages: 1, headless: browser.headless },
      site: site.id,
      url,
    };
  };

  handlers.inspect = async (params) => {
    await ensureBrowser((params && params.profile) || 'default', true);
    await ensurePage();
    try { return await evalJs(E.inspect); } catch (e) { return { error: String(e.message || e) }; }
  };

  handlers.eval = async (params) => {
    if (!browser) throw new Error('browser not running');
    return evalJs(params && params.expression);
  };

  handlers.login = async (params) => {
    const profile = (params && params.profile) || 'default';
    const timeout = Math.min(Number(params && params.timeoutMs) || 300000, 330000);
    await ensureBrowser(profile, false);
    await navigate(site.url);
    const start = Date.now();
    let last = {};
    while (Date.now() - start < timeout) {
      try {
        last = await evalJs(E.state);
        if (last.hasInput && !last.needsLogin) return { ok: true, loggedIn: true, url: last.url };
      } catch (e) { /* keep waiting */ }
      await sleep(1500);
    }
    return { ok: false, loggedIn: false, url: last.url || '', needsLogin: !!last.needsLogin };
  };

  handlers.streamAsk = async (params) => {
    const question = params && params.question;
    if (!question || !String(question).trim()) throw new Error('question required');
    const streamId = 's' + (++streamSeq);
    const state = { stopped: false };
    streams.set(streamId, state);

    (async () => {
      try {
        const profile = (params && params.profile) || 'default';
        const headless = params && params.headless !== undefined ? params.headless !== false : true;
        await ensureBrowser(profile, headless);
        await ensurePage();
        await prepare();

        if (params && params.reset === true) {
          await resetChat();
          await prepare();
        }

        if (params && params.model && site.applyModel) {
          log('switch model -> ' + params.model);
          await site.applyModel({ evalJs, sleep, log, navigate }, params.model);
          await sleep(500);
        }

        const preText = await evalJs(E.answer).catch(() => '') || '';
        const beforeCount = await evalJs(E.assistantCount);
        const payload = ((params.toolsText && String(params.toolsText).trim()) ? String(params.toolsText).trim() + '\n\n' : '') + String(question);
        log('sending question chars=' + payload.length);
        await sendQuestion(payload);

        const timeoutMs = Math.min(Number(params && params.timeoutMs) || 120000, 300000);
        const deadline = Date.now() + timeoutMs;
        let last = preText;
        let lastChange = Date.now();
        while (Date.now() < deadline && !state.stopped) {
          let text = preText;
          let gen = true;
          let count = beforeCount;
          try { text = await evalJs(E.answer); } catch (e) { /* polling */ }
          try { gen = await evalJs(E.generating); } catch (e) { /* assume generating */ }
          try { count = await evalJs(E.assistantCount); } catch (e) { /* ignore */ }
          if (E.failure && await evalJs(E.failure).catch(() => false)) {
            throw new Error(site.failureError || 'upstream rejected the message');
          }
          if (text !== last) { last = text; lastChange = Date.now(); }
          const fresh = count > beforeCount || text !== preText;
          if (fresh && last && !gen && Date.now() - lastChange >= 900) break;
          if (!gen && Date.now() - lastChange >= 20000) break;
          await sleep(350);
        }

        if (state.stopped) throw new Error('stopped');
        const result = cleanText(last);
        const toolCalls = parseToolCalls(result, params && params.tools);
        log('reply chars=' + result.length + ' toolCalls=' + toolCalls.length);
        emit('stream-end', { streamId, ok: true, result, ...(toolCalls.length ? { toolCalls } : {}) });
      } catch (err) {
        emit('stream-end', { streamId, ok: false, error: String((err && err.message) || err) });
      } finally {
        streams.delete(streamId);
      }
    })();

    return { streamId };
  };

  handlers.streamStop = async (params) => {
    const st = streams.get(params && params.streamId);
    if (st) st.stopped = true;
    return { stopped: !!st };
  };

  handlers.shutdown = async () => {
    await closeBrowser();
    setTimeout(() => process.exit(0), 100);
    return { ok: true };
  };

  function start() {
    let stdinBuf = Buffer.alloc(0);
    process.stdin.on('data', (chunk) => {
      stdinBuf = Buffer.concat([stdinBuf, chunk]);
      for (;;) {
        const i = stdinBuf.indexOf('\n');
        if (i < 0) break;
        const line = stdinBuf.subarray(0, i).toString('utf8').trim();
        stdinBuf = stdinBuf.subarray(i + 1);
        if (!line) continue;
        let msg;
        try { msg = JSON.parse(line); } catch (e) { logErr('bad rpc line', e.message); continue; }
        if (msg && msg.id !== undefined && msg.method) {
          const h = handlers[msg.method];
          if (!h) { respond(msg.id, false, null, new Error('unknown method: ' + msg.method)); continue; }
          Promise.resolve()
            .then(() => h(msg.params || {}, msg))
            .then((res) => respond(msg.id, true, res === undefined ? null : res))
            .catch((err) => respond(msg.id, false, null, err));
        }
      }
    });

    process.stdin.on('end', async () => { await closeBrowser(); process.exit(0); });
    process.on('SIGINT', async () => { await closeBrowser(); process.exit(0); });
    process.on('SIGTERM', async () => { await closeBrowser(); process.exit(0); });
    process.on('uncaughtException', (e) => logErr('uncaught', e.stack || e.message));
    process.on('unhandledRejection', (e) => logErr('unhandled rejection', (e && (e.stack || e.message)) || e));

    log('driver ready ' + VERSION + ' site=' + site.id + ' base=' + BASE);
    emit('ready', { version: VERSION, site: site.id, baseDir: BASE, pid: process.pid });
  }

  return { start, evalJs, sleep, log };
}

module.exports = { createRuntime, VERSION };
