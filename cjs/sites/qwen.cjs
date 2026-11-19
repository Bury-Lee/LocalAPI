const NOT_READY = /\u672a\u9009\u62e9\u6a21\u578b|\u6a21\u578b\u52a0\u8f7d\u4e2d/;

const E = {
  state: `(() => {
    const url = location.href;
    const body = (document.body && document.body.innerText) || '';
    const visible = (e) => { const r = e.getBoundingClientRect(); return r.width > 0 && r.height > 0; };
    const btns = Array.from(document.querySelectorAll('button, a, [role="button"]'));
    const hasLoginBtn = btns.some((e) => visible(e) && /^(登\\s*录|sign\\s*in)$/i.test((e.innerText || '').trim()));
    const hasRegisterBtn = btns.some((e) => visible(e) && /^(注\\s*册|sign\\s*up)$/i.test((e.innerText || '').trim()));
    return {
      url,
      needsLogin: url.includes('/auth') || url.includes('/login') || url.includes('/signin') || !!document.querySelector('input[type="password"]') || (hasLoginBtn && hasRegisterBtn),
      hasInput: !!document.querySelector('textarea.message-input-textarea, textarea, [contenteditable="true"]'),
      body: body.slice(0, 300),
    };
  })()`,

  modal: `(() => {
    const m = document.querySelector('.qwen-chat-v2-modal');
    if (!m) return false;
    const cs = getComputedStyle(m);
    return cs.display !== 'none' && cs.visibility !== 'hidden';
  })()`,

  dismiss: `(() => {
    const m = document.querySelector('.qwen-chat-v2-modal');
    if (!m) return false;
    const inp = m.querySelector('input');
    if (inp) {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value');
      if (setter && setter.set) setter.set.call(inp, inp.value || '2000');
      inp.dispatchEvent(new Event('input', { bubbles: true }));
      inp.dispatchEvent(new Event('change', { bubbles: true }));
    }
    const btns = Array.from(m.querySelectorAll('button')).filter((b) => {
      const r = b.getBoundingClientRect();
      return r.width > 0 && r.height > 0 && getComputedStyle(b).display !== 'none';
    });
    const btn = btns[btns.length - 1];
    if (btn) { btn.click(); return true; }
    return false;
  })()`,

  badge: `(() => {
    const el = document.querySelector('.wms-trigger__content') || document.querySelector('.wms-trigger');
    return el ? ((el.innerText || el.textContent || '').trim()) : '';
  })()`,

  clickModel: `(() => { const el = document.querySelector('.wms-trigger'); if (!el) return false; el.click(); return true; })()`,

  pickModel: `(() => {
    const badge = ((document.querySelector('.wms-trigger__content') || {}).innerText || '').trim();
    const els = Array.from(document.querySelectorAll('.qwen-chat-v2-dropdown-menu-select, [class*="dropdown-menu"] li, [class*="dropdown-menu"] [role="option"]'));
    for (const el of els) {
      const t = (el.innerText || '').trim();
      if (!t || t === badge) continue;
      if (!/qwen/i.test(t)) continue;
      const r = el.getBoundingClientRect();
      if (r.width > 0 && r.height > 0 && getComputedStyle(el).display !== 'none') { el.click(); return t; }
    }
    return '';
  })()`,

  input: `(() => {
    const el = document.querySelector('textarea.message-input-textarea') || document.querySelector('textarea');
    if (!el) return false;
    const r = el.getBoundingClientRect();
    return r.width > 0 && r.height > 0;
  })()`,

  fill: (text) => `(() => {
    const el = document.querySelector('textarea.message-input-textarea') || document.querySelector('textarea');
    if (!el) return false;
    el.focus();
    const proto = window.HTMLTextAreaElement ? HTMLTextAreaElement.prototype : Object.getPrototypeOf(el);
    const setter = Object.getOwnPropertyDescriptor(proto, 'value');
    if (setter && setter.set) setter.set.call(el, ${JSON.stringify(text)});
    else { el.value = ${JSON.stringify(text)}; }
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
    return true;
  })()`,

  send: `(() => {
    const sels = ['button.send-button', '.chat-prompt-send-button', '[class*="send-button"]', '[class*="sendButton"]', 'button[aria-label*="send" i]'];
    for (const s of sels) {
      const el = document.querySelector(s);
      if (el) {
        const r = el.getBoundingClientRect();
        if (r.width > 0 && r.height > 0 && !el.disabled) { el.click(); return true; }
      }
    }
    return false;
  })()`,

  userCount: `document.querySelectorAll('.chat-user-message').length`,
  assistantCount: `document.querySelectorAll('.chat-response-message').length`,

  generating: `(() => {
    for (const s of ['.stop-button', '.response-loading']) {
      const el = document.querySelector(s);
      if (el) {
        const cs = getComputedStyle(el);
        if (cs.display !== 'none' && cs.visibility !== 'hidden' && cs.opacity !== '0') return true;
      }
    }
    return false;
  })()`,

  answer: `(() => {
    function walk(node, out) {
      if (!node) return;
      if (node.nodeType === 3) { out.push(node.textContent); return; }
      if (node.nodeType !== 1) return;
      const tag = node.tagName.toLowerCase();
      if (tag === 'pre') {
        const code = node.querySelector('code');
        const lang = ((code && code.className || '').match(/language-(\\S+)/) || [])[1] || '';
        out.push('\\n\`\`\`' + lang + '\\n' + ((code ? code.textContent : node.textContent) || '') + '\\n\`\`\`\\n');
        return;
      }
      if (tag === 'code') {
        const pt = node.parentElement ? node.parentElement.tagName.toLowerCase() : '';
        if (pt !== 'pre') out.push('\`' + (node.textContent || '') + '\`');
        return;
      }
      for (const ch of node.childNodes) walk(ch, out);
      if (['p','div','li','br','h1','h2','h3','h4','h5','h6'].includes(tag)) out.push('\\n');
    }
    const els = document.querySelectorAll('.response-message-content');
    if (!els.length) return '';
    const out = [];
    walk(els[els.length - 1], out);
    return out.join('').trim();
  })()`,

  newChat: `(() => {
    const sels = ['.new-chat', '[class*="new-chat"]', '[class*="newChat"]'];
    for (const s of sels) {
      const el = document.querySelector(s);
      if (el) {
        const r = el.getBoundingClientRect();
        if (r.width > 0 && r.height > 0) { el.click(); return true; }
      }
    }
    return false;
  })()`,

  inspect: `(() => {
    const url = location.href;
    const body = (document.body && document.body.innerText) || '';
    const visible = (e) => { const r = e.getBoundingClientRect(); return r.width > 0 && r.height > 0; };
    const btns = Array.from(document.querySelectorAll('button, a, [role="button"]'));
    const hasLoginBtn = btns.some((e) => visible(e) && /^(登\\s*录|sign\\s*in)$/i.test((e.innerText || '').trim()));
    const hasRegisterBtn = btns.some((e) => visible(e) && /^(注\\s*册|sign\\s*up)$/i.test((e.innerText || '').trim()));
    return {
      login: {
        needsLogin: url.includes('/auth') || url.includes('/login') || !!document.querySelector('input[type="password"]') || (hasLoginBtn && hasRegisterBtn),
        hasChatInput: !!document.querySelector('textarea.message-input-textarea, textarea'),
        url,
      },
      modelBadge: { text: ((document.querySelector('.wms-trigger__content') || {}).innerText || '').trim() },
      body: body.slice(0, 300),
    };
  })()`,
};

async function prepare({ evalJs, sleep, settleModal }) {
  const deadline = Date.now() + 60000;
  let modelTries = 0;
  while (Date.now() < deadline) {
    const st = await evalJs(E.state);
    if (st.needsLogin) throw new Error('login required: open /login, sign in, then retry');
    if (await settleModal()) continue;
    const badge = await evalJs(E.badge);
    if (NOT_READY.test(badge) && modelTries < 3) {
      modelTries++;
      await evalJs(E.clickModel);
      await sleep(700);
      await evalJs(E.pickModel);
      await sleep(700);
      continue;
    }
    if (await evalJs(E.input)) {
      if (!badge) await sleep(500);
      return st;
    }
    await sleep(500);
  }
  throw new Error('qwen chat page not ready');
}

const MODELS = [
  { id: 'qwen3.7-plus', name: 'Qwen 3.7 Plus', label: 'Qwen3.7-Plus' },
  { id: 'qwen3.8-max', name: 'Qwen 3.8 Max', label: 'Qwen3.8-Max' },
];

async function applyModel({ evalJs, sleep, log }, modelId) {
  const hit = MODELS.find((m) => m.id === modelId || m.label.toLowerCase() === String(modelId).toLowerCase());
  const label = hit ? hit.label : String(modelId);
  for (let attempt = 0; attempt < 2; attempt++) {
    await evalJs(E.clickModel);
    await sleep(900);
    const picked = await evalJs(`(() => {
      const els = Array.from(document.querySelectorAll('.qwen-chat-v2-dropdown-menu-select, [role="option"], [class*="dropdown-menu"] li'));
      const lab = ${JSON.stringify(label)}.toLowerCase();
      for (const el of els) {
        const t = (el.innerText || '').trim();
        if (!t.toLowerCase().startsWith(lab)) continue;
        const r = el.getBoundingClientRect();
        if (r.width > 0 && r.height > 0) { el.click(); return t.split('\\n')[0]; }
      }
      return '';
    })()`);
    if (!picked) {
      log('model option not found: ' + label + ' (attempt ' + (attempt + 1) + ')');
      await sleep(600);
      continue;
    }
    let badge = '';
    for (let i = 0; i < 20; i++) {
      badge = await evalJs(E.badge);
      if (badge.toLowerCase().includes(label.toLowerCase())) break;
      await sleep(500);
    }
    if (badge.toLowerCase().includes(label.toLowerCase())) {
      log('model switched to ' + picked + ' (badge=' + badge + ')');
      return;
    }
    log('model badge did not change after picking ' + picked + ' (badge=' + badge + '), retry');
    await sleep(500);
  }
}

module.exports = {
  id: 'qwen',
  url: 'https://chat.qwen.ai/',
  hostMatch: 'qwen.ai',
  loginError: 'login required: open /login, sign in, then retry',
  readyError: 'qwen chat page not ready',
  exprs: E,
  prepare,
  models: MODELS,
  applyModel,
};
