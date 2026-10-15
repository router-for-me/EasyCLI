try {
  if (location.origin === O) {
    var auth;
    try { auth = JSON.parse(localStorage.getItem('cli-proxy-auth')); } catch (e) {}
    if (!auth || typeof auth !== 'object' || Array.isArray(auth)) auth = {};
    if (!auth.state || typeof auth.state !== 'object' || Array.isArray(auth.state)) auth.state = {};
    auth.state.managementKey = K;
    localStorage.setItem('cli-proxy-auth', JSON.stringify(auth));
  }
} catch (e) {}
try {
  if (window.top !== window || !/^https?:$/.test(location.protocol)) return;
  var mount = function () {
    var root = document.documentElement;
    if (!root || root.querySelector('cpa-plugin-controls')) return;
    var STORE = 'cpa-gui.plugin-controls';
    var DEFAULT = { h: 'right', v: 'bottom', x: 8, y: 8 };
    var pos = { h: DEFAULT.h, v: DEFAULT.v, x: DEFAULT.x, y: DEFAULT.y };
    var host = document.createElement('cpa-plugin-controls');
    var shadow = host.attachShadow({ mode: 'closed' });
    var scheme = window.matchMedia('(prefers-color-scheme: dark)');
    var bar = document.createElement('div');
    var raised = false;
    var focused = false;
    var down = null;
    var dragging = false;
    var suppress = false;
    try {
      var saved = JSON.parse(localStorage.getItem(STORE));
      if (saved && (saved.h === 'left' || saved.h === 'right') && (saved.v === 'top' || saved.v === 'bottom') && isFinite(saved.x) && isFinite(saved.y)) pos = { h: saved.h, v: saved.v, x: +saved.x, y: +saved.y };
    } catch (e) {}
    var clamp = function (value, max) { return Math.min(Math.max(0, value), Math.max(0, max)); };
    var apply = function () {
      var box = bar.getBoundingClientRect();
      var width = document.documentElement.clientWidth;
      var height = document.documentElement.clientHeight;
      pos.x = clamp(pos.x, width - box.width);
      pos.y = clamp(pos.y, height - box.height);
      host.style.left = pos.h === 'left' ? pos.x + 'px' : 'auto';
      host.style.right = pos.h === 'right' ? pos.x + 'px' : 'auto';
      host.style.top = pos.v === 'top' ? pos.y + 'px' : 'auto';
      host.style.bottom = pos.v === 'bottom' ? pos.y + 'px' : 'auto';
    };
    var place = function (left, top) {
      var box = bar.getBoundingClientRect();
      var width = document.documentElement.clientWidth;
      var height = document.documentElement.clientHeight;
      left = clamp(left, width - box.width);
      top = clamp(top, height - box.height);
      pos.h = left + box.width / 2 < width / 2 ? 'left' : 'right';
      pos.v = top + box.height / 2 < height / 2 ? 'top' : 'bottom';
      pos.x = pos.h === 'left' ? left : width - left - box.width;
      pos.y = pos.v === 'top' ? top : height - top - box.height;
      apply();
    };
    var save = function () {
      try { localStorage.setItem(STORE, JSON.stringify(pos)); } catch (e) {}
    };
    var paint = function () {
      var dark = scheme.matches;
      bar.style.background = dark ? 'rgba(32,34,40,0.92)' : 'rgba(255,255,255,0.92)';
      bar.style.border = '1px solid ' + (dark ? 'rgba(255,255,255,0.18)' : 'rgba(0,0,0,0.18)');
      bar.style.color = dark ? '#e8eaf0' : '#2b2f38';
      bar.style.opacity = raised || focused || dragging ? '1' : '0.35';
    };
    var control = function (label, shapes, extra) {
      var button = document.createElement('button');
      var svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
      button.type = 'button';
      button.setAttribute('aria-label', label);
      button.title = label;
      Object.assign(button.style, { display: 'flex', alignItems: 'center', justifyContent: 'center', width: '24px', height: '24px', margin: '0', padding: '0', border: '0', borderRadius: '50%', background: 'transparent', color: 'inherit', cursor: 'pointer' }, extra || {});
      svg.setAttribute('viewBox', '0 0 24 24');
      svg.setAttribute('width', '16');
      svg.setAttribute('height', '16');
      svg.setAttribute('fill', 'none');
      svg.setAttribute('stroke', 'currentColor');
      svg.setAttribute('stroke-width', '2');
      svg.setAttribute('stroke-linecap', 'round');
      svg.setAttribute('stroke-linejoin', 'round');
      shapes.forEach(function (shape) {
        var node = document.createElementNS('http://www.w3.org/2000/svg', shape[0]);
        Object.keys(shape[1]).forEach(function (name) { node.setAttribute(name, shape[1][name]); });
        svg.appendChild(node);
      });
      button.appendChild(svg);
      button.addEventListener('mouseenter', function () { button.style.background = 'rgba(127,127,127,0.25)'; });
      button.addEventListener('mouseleave', function () { button.style.background = 'transparent'; });
      bar.appendChild(button);
      return button;
    };
    var path = function (d) { return [['path', { d: d }]]; };
    var dots = [6, 12, 18].map(function (cy) { return ['circle', { cx: '9', cy: String(cy), r: '1', fill: 'currentColor' }]; })
      .concat([6, 12, 18].map(function (cy) { return ['circle', { cx: '15', cy: String(cy), r: '1', fill: 'currentColor' }]; }));
    var grip = control(L.move, dots, { width: '14px', borderRadius: '999px', cursor: 'grab' });
    var back = control(L.back, path('M15 18l-6-6 6-6'));
    var reload = control(L.reload, path('M21 12a9 9 0 1 1-3-6.7L21 8M21 3v5h-5'));
    var open = control(L.open, path('M15 3h6v6M10 14L21 3M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6'));
    back.addEventListener('click', function () { history.back(); });
    reload.addEventListener('click', function () { location.reload(); });
    open.addEventListener('click', function () { window.open(location.href, '_blank'); });
    Object.assign(host.style, { all: 'initial', position: 'fixed', zIndex: '2147483647' });
    Object.assign(bar.style, { display: 'flex', alignItems: 'center', gap: '2px', padding: '3px', borderRadius: '999px', transition: 'opacity 150ms ease', touchAction: 'none', userSelect: 'none', webkitUserSelect: 'none' });
    grip.addEventListener('dblclick', function () {
      pos = { h: DEFAULT.h, v: DEFAULT.v, x: DEFAULT.x, y: DEFAULT.y };
      apply();
      try { localStorage.removeItem(STORE); } catch (e) {}
    });
    grip.addEventListener('keydown', function (event) {
      var step = event.shiftKey ? 40 : 8;
      var move = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, -step], ArrowDown: [0, step] }[event.key];
      if (!move) return;
      event.preventDefault();
      var box = bar.getBoundingClientRect();
      place(box.left + move[0], box.top + move[1]);
      save();
    });
    bar.addEventListener('pointerdown', function (event) {
      if (event.button !== 0) return;
      var box = bar.getBoundingClientRect();
      down = { id: event.pointerId, x: event.clientX, y: event.clientY, left: box.left, top: box.top };
      window.addEventListener('pointermove', move, true);
      window.addEventListener('pointerup', release, true);
      window.addEventListener('pointercancel', release, true);
    });
    var move = function (event) {
      if (!down || event.pointerId !== down.id) return;
      var dx = event.clientX - down.x;
      var dy = event.clientY - down.y;
      if (!dragging) {
        if (Math.sqrt(dx * dx + dy * dy) < 4) return;
        dragging = true;
        try { bar.setPointerCapture(event.pointerId); } catch (e) {}
        paint();
      }
      place(down.left + dx, down.top + dy);
    };
    var release = function (event) {
      if (!down || event.pointerId !== down.id) return;
      window.removeEventListener('pointermove', move, true);
      window.removeEventListener('pointerup', release, true);
      window.removeEventListener('pointercancel', release, true);
      if (dragging) {
        suppress = true;
        setTimeout(function () { suppress = false; }, 0);
        save();
      }
      down = null;
      dragging = false;
      paint();
    };
    bar.addEventListener('click', function (event) {
      if (!suppress) return;
      suppress = false;
      event.stopPropagation();
      event.preventDefault();
    }, true);
    bar.addEventListener('mouseenter', function () { raised = true; paint(); });
    bar.addEventListener('mouseleave', function () { raised = false; paint(); });
    bar.addEventListener('focusin', function (event) { focused = event.target.matches(':focus-visible'); paint(); });
    bar.addEventListener('focusout', function () { focused = false; paint(); });
    if (scheme.addEventListener) scheme.addEventListener('change', paint); else scheme.addListener(paint);
    window.addEventListener('resize', apply);
    paint();
    shadow.appendChild(bar);
    root.appendChild(host);
    apply();
  };
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', mount); else mount();
} catch (e) {}
