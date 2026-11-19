const INPUT_SELECTOR = "div.tiptap.ProseMirror[contenteditable='true'], textarea[data-testid='chat_input_input'], div[contenteditable='true'][data-testid='chat_input_input'], #chat-input";

const E = {
  state: `(() => {
    const url = location.href;
    const body = (document.body && document.body.innerText) || '';
    const input = !!document.querySelector(${JSON.stringify(INPUT_SELECTOR)});
    const guest = !input && /\\u767b\\u5f55|\\u626b\\u7801|sign in/i.test(body) && !/\\u9000\\u51fa\\u767b\\u5f55/.test(body);
    return {
      url,
      needsLogin: url.includes('/login') || !!document.querySelector('input[type="password"]') || guest,
      hasInput: input,
      body: body.slice(0, 300),
    };
  })()`,

  input: `(() => {
    const el = document.querySelector(${JSON.stringify(INPUT_SELECTOR)});
    if (!el) return false;
    const r = el.getBoundingClientRect();
    return r.width > 0 && r.height > 0;
  })()`,

  fill: (text) => `(() => {
    const el = document.querySelector(${JSON.stringify(INPUT_SELECTOR)});
    if (!el) return false;
    el.focus();
    if (el.tagName.toLowerCase() === 'textarea') {
      const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value');
      if (setter && setter.set) setter.set.call(el, ${JSON.stringify(text)});
      else el.value = ${JSON.stringify(text)};
      el.dispatchEvent(new Event('input', { bubbles: true }));
      el.dispatchEvent(new Event('change', { bubbles: true }));
    } else {
      document.execCommand('selectAll', false, null);
      document.execCommand('delete', false, null);
      document.execCommand('insertText', false, ${JSON.stringify(text)});
      el.dispatchEvent(new InputEvent('input', { bubbles: true, data: ${JSON.stringify(text)} }));
    }
    return true;
  })()`,

  send: `(() => {
    const sels = ['[data-testid="chat_input_send_button"]', '#flow-end-msg-send', 'button[aria-label*="\\u53d1\\u9001"]', '[class*="send-btn"]'];
    for (const s of sels) {
      const el = document.querySelector(s);
      if (el) {
        const r = el.getBoundingClientRect();
        if (r.width > 0 && r.height > 0 && !el.disabled) { el.click(); return true; }
      }
    }
    const input = document.querySelector(${JSON.stringify(INPUT_SELECTOR)});
    if (input) {
      const ev = { key: 'Enter', code: 'Enter', keyCode: 13, which: 13, bubbles: true };
      input.dispatchEvent(new KeyboardEvent('keydown', ev));
      input.dispatchEvent(new KeyboardEvent('keypress', ev));
      input.dispatchEvent(new KeyboardEvent('keyup', ev));
      return true;
    }
    return false;
  })()`,

  userCount: `document.querySelectorAll('[data-testid="send_message"]').length`,

  assistantCount: `document.querySelectorAll('[data-testid="receive_message"]').length`,

  failure: `!!document.querySelector('[data-testid="message_box_failed_icon"]')`,

  generating: `(() => {
    for (const s of ['[data-testid="message_loading"]', '[data-testid="conversation-list-v2-status-loading"]', '[data-testid="chat_input_stop_button"]']) {
      const el = document.querySelector(s);
      if (el) {
        const cs = getComputedStyle(el);
        if (cs.display !== 'none' && cs.visibility !== 'hidden') return true;
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
        out.push('\\n\`\`\`\\n' + ((code ? code.textContent : node.textContent) || '') + '\\n\`\`\`\\n');
        return;
      }
      for (const ch of node.childNodes) walk(ch, out);
      if (['p','div','li','br','h1','h2','h3','h4','h5','h6'].includes(tag)) out.push('\\n');
    }
    const els = document.querySelectorAll('[data-testid="receive_message"] [data-testid="message_text_content"]');
    let target = els.length ? els[els.length - 1] : null;
    if (!target) {
      const all = document.querySelectorAll('[data-testid="receive_message"]');
      target = all.length ? all[all.length - 1] : null;
    }
    if (!target) return '';
    const out = [];
    walk(target, out);
    return out.join('').trim();
  })()`,

  newChat: `(() => {
    const sels = ['[data-testid="create_new_chat_button"]', '[data-testid="new_chat_button"]', '[class*="new-chat"]', '[class*="newChat"]'];
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
    return {
      login: {
        needsLogin: url.includes('/login') || !!document.querySelector('input[type="password"]'),
        hasChatInput: !!document.querySelector(${JSON.stringify(INPUT_SELECTOR)}),
        url,
      },
      modelBadge: { text: ((document.querySelector('[class*="model"] [class*="name"]') || {}).innerText || '').trim() },
      body: body.slice(0, 300),
    };
  })()`,
};

async function prepare({ evalJs, sleep }) {
  const deadline = Date.now() + 45000;
  while (Date.now() < deadline) {
    const st = await evalJs(E.state);
    if (st.hasInput && (await evalJs(E.input))) return st;
    if (st.needsLogin && !st.hasInput) throw new Error('doubao login required: open /login once, sign in, then retry');
    await sleep(600);
  }
  throw new Error('doubao chat page not ready (login may be required)');
}

const MODELS = [
  { id: 'doubao-pro', name: '豆包 快速', label: '快速' },
  { id: 'doubao-think', name: '豆包 深度思考', label: '深度思考' },
  { id: 'doubao-expert', name: '豆包 专家', label: '专家' },
];

async function applyModel({ evalJs, sleep, log }, modelId) {
  const hit = MODELS.find((m) => m.id === modelId || m.label === String(modelId));
  const label = hit ? hit.label : String(modelId);
  const opened = await evalJs(`(() => {
    const els = Array.from(document.querySelectorAll('button, [role="button"], [role="menuitem"], [class*="mode"], [class*="model"], [class*="select"]'));
    for (const el of els) {
      const t = (el.innerText || '').trim();
      if (!/^(快速|深度思考|专家|专业)/.test(t)) continue;
      const r = el.getBoundingClientRect();
      if (r.width > 0 && r.height > 0) { el.click(); return t.slice(0, 20); }
    }
    return '';
  })()`);
  await sleep(800);
  const picked = await evalJs(`(() => {
    const els = Array.from(document.querySelectorAll('[role="menuitem"], [role="option"], [role="button"], button, li, div[class*="item"]'));
    const lab = ${JSON.stringify(label)};
    for (const el of els) {
      const t = (el.innerText || '').trim();
      if (!t.startsWith(lab)) continue;
      const r = el.getBoundingClientRect();
      if (r.width > 0 && r.height > 0) { el.click(); return t.split('\\n')[0].slice(0, 30); }
    }
    return '';
  })()`);
  if (!picked) { log('model option not found: ' + label + ' (opened=' + opened + ')'); return; }
  await sleep(600);
  log('model switched to ' + picked);
}

module.exports = {
  id: 'doubao',
  url: 'https://www.doubao.com/chat/',
  hostMatch: 'doubao.com',
  loginError: 'doubao login required: open /login (or log in once in the profile), then retry',
  readyError: 'doubao chat page not ready (login may be required)',
  failureError: 'doubao rejected the message (login or quota required); open /login and sign in',
  exprs: E,
  prepare,
  models: MODELS,
  applyModel,
};
