const COMPOSER_SELECTORS = [
  'div[role="textbox"]#prompt-textarea',
  'div[role="textbox"].ProseMirror',
  'textarea#prompt-textarea',
  'textarea#mobile-composer-prompt',
  'form textarea',
  'textarea[placeholder]',
];

const PICK_COMPOSER = `(() => {
  const sels = ${JSON.stringify(COMPOSER_SELECTORS)};
  for (const s of sels) {
    for (const el of document.querySelectorAll(s)) {
      const r = el.getBoundingClientRect();
      if (r.width > 0 && r.height > 0) return el;
    }
  }
  return null;
})()`;

const E = {
  state: `(() => {
    const url = location.href;
    const body = (document.body && document.body.innerText) || '';
    return {
      url,
      needsLogin: url.includes('/auth/login') || url.includes('/auth/') || !!document.querySelector('input[type="password"]'),
      hasInput: (${PICK_COMPOSER}) !== null,
      body: body.slice(0, 300),
    };
  })()`,

  input: `(() => (${PICK_COMPOSER}) !== null)()`,

  focus: `(() => {
    const el = ${PICK_COMPOSER};
    if (!el) return false;
    el.focus();
    if (el.tagName && el.tagName.toLowerCase() === 'textarea') el.select();
    else document.execCommand('selectAll', false, null);
    return true;
  })()`,

  rect: `(() => {
    const el = ${PICK_COMPOSER};
    if (!el) return null;
    const r = el.getBoundingClientRect();
    return { x: Math.round(r.left + r.width / 2), y: Math.round(r.top + r.height / 2) };
  })()`,

  fill: (text) => `(() => {
    const el = ${PICK_COMPOSER};
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
    const sels = ['button[aria-label*="Send" i]:not([data-testid="stop-button"])', 'button[data-testid="send-button"]', 'form:has(#prompt-textarea) button[type="submit"]', 'form:has(textarea#mobile-composer-prompt) button[type="submit"]', '[data-testid="composer-submit-button"]', 'button[type="submit"]'];
    for (const s of sels) {
      let list = [];
      try { list = Array.from(document.querySelectorAll(s)); } catch (e) { list = []; }
      for (const el of list) {
        const r = el.getBoundingClientRect();
        if (r.width > 0 && r.height > 0 && !el.disabled && el.getAttribute('aria-disabled') !== 'true') { el.click(); return true; }
      }
    }
    return false;
  })()`,

  userCount: `document.querySelectorAll('ol > li[id]').length`,
  assistantCount: `document.querySelectorAll('ol > li[id]').length`,

  generating: `(() => {
    for (const s of ['[data-testid="stop-button"]', 'button[aria-label*="Stop" i]', 'button[aria-label*="\\u505c\\u6b62"]', '.result-streaming']) {
      const el = document.querySelector(s);
      if (el) {
        const cs = getComputedStyle(el);
        if (cs.display !== 'none' && cs.visibility !== 'hidden') return true;
      }
    }
    return false;
  })()`,

  sent: `(() => {
    const el = ${PICK_COMPOSER};
    const val = el ? (el.value !== undefined ? el.value : el.innerText) : '';
    if (!val || !String(val).trim()) return true;
    return !!document.querySelector('[data-testid="stop-button"], button[aria-label*="Stop" i], button[aria-label*="\\u505c\\u6b62"]');
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
    const turns = document.querySelectorAll('ol > li[id]');
    if (!turns.length) return '';
    const last = turns[turns.length - 1];
    const md = last.querySelector('.markdown');
    const scope = md || last;
    const out = [];
    walk(scope, out);
    return out.join('')
      .replace(/^(?:\\u590d\\u5236\\u56de\\u590d|\\u5206\\u4eab|\\u590d\\u5236|\\u7f16\\u8f91|Retry|\\u91cd\\u65b0\\u751f\\u6210)\\s*$/gm, '')
      .trim();
  })()`,

  inspect: `(() => {
    const url = location.href;
    const body = (document.body && document.body.innerText) || '';
    return {
      login: {
        needsLogin: url.includes('/auth/login') || !!document.querySelector('input[type="password"]'),
        hasChatInput: (${PICK_COMPOSER}) !== null,
        url,
      },
      modelBadge: { text: ((document.querySelector('[data-testid="model-switcher-dropdown"]') || {}).innerText || '').trim() },
      body: body.slice(0, 300),
    };
  })()`,
};

const MODELS = [
  { id: 'auto', name: 'ChatGPT Auto', label: 'Auto' },
  { id: 'gpt-5-5', name: 'GPT-5.5', label: 'GPT-5.5' },
  { id: 'gpt-5-4-thinking', name: 'GPT-5.4 Thinking', label: 'GPT-5.4 Thinking' },
  { id: 'gpt-5-3-mini', name: 'GPT-5.3 Mini', label: 'GPT-5.3 Mini' },
  { id: 'gpt-5-mini', name: 'GPT-5 mini', label: 'GPT-5 mini' },
];

async function applyModel({ evalJs, sleep, log, navigate }, modelId) {
  const hit = MODELS.find((m) => m.id === modelId || m.label.toLowerCase() === String(modelId).toLowerCase());
  const label = hit ? hit.label : String(modelId);
  const opened = await evalJs(`(() => {
    const sels = ['[data-testid="model-switcher-dropdown"]', 'button[aria-haspopup="menu"]', 'button[aria-label*="model" i]'];
    for (const s of sels) {
      const el = document.querySelector(s);
      if (el) {
        const r = el.getBoundingClientRect();
        if (r.width > 0 && r.height > 0) { el.click(); return true; }
      }
    }
    return false;
  })()`);
  await sleep(800);
  const picked = await evalJs(`(() => {
    const els = Array.from(document.querySelectorAll('[role="menuitemradio"], [role="menuitem"], [role="option"]'));
    const lab = ${JSON.stringify(label)}.toLowerCase();
    for (const el of els) {
      const t = (el.innerText || '').trim();
      if (!t.toLowerCase().startsWith(lab)) continue;
      const r = el.getBoundingClientRect();
      if (r.width > 0 && r.height > 0) { el.click(); return t.split('\\n')[0]; }
    }
    return '';
  })()`);
  if (picked) {
    await sleep(500);
    log('model switched to ' + picked);
    return;
  }
  log('model option not found (opened=' + opened + '), falling back to ?model=' + modelId);
  await navigate('https://chatgpt.com/?model=' + encodeURIComponent(modelId));
  await sleep(1500);
}

module.exports = {
  id: 'gpt',
  url: 'https://chatgpt.com/',
  hostMatch: 'chatgpt.com',
  loginError: 'chatgpt login required: open /login, sign in, then retry',
  readyError: 'chatgpt composer not found (login or Cloudflare challenge may be blocking)',
  cdpInput: true,
  exprs: E,
  models: MODELS,
  applyModel,
};
