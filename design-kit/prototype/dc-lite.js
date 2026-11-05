/*
 * dc-lite.js — a tiny runtime that renders the prototype's .dc.html templates
 * in any browser, offline. Reference only: the real app is React + TypeScript.
 *
 * Supports: {{dotted.path}} holes, <sc-if value>, <sc-for list as>, event
 * attributes bound to functions (onClick, onChange), boolean attributes,
 * and a DCLogic base class with state, setState and componentDidMount.
 */
(function () {
  'use strict';

  class DCLogic {
    constructor(props) { this.props = props || {}; this.state = {}; }
    setState(patch) {
      const next = typeof patch === 'function' ? patch(this.state, this.props) : patch;
      this.state = Object.assign({}, this.state, next);
      this.__render && this.__render();
    }
    forceUpdate() { this.__render && this.__render(); }
  }
  window.DCLogic = DCLogic;

  const WHOLE = /^\s*\{\{([^}]+)\}\}\s*$/;
  const ANY = /\{\{([^}]+)\}\}/g;
  const SVG_NS = 'http://www.w3.org/2000/svg';

  function lookup(path, scopes) {
    path = path.trim();
    if (path === 'true') return true;
    if (path === 'false') return false;
    if (path === 'null') return null;
    if (/^-?\d+(\.\d+)?$/.test(path)) return Number(path);
    const parts = path.split('.');
    for (const scope of scopes) {
      if (scope && Object.prototype.hasOwnProperty.call(scope, parts[0])) {
        let v = scope[parts[0]];
        for (const p of parts.slice(1)) { if (v == null) return undefined; v = v[p]; }
        return v;
      }
    }
    return undefined;
  }

  function interpolate(str, scopes) {
    return str.replace(ANY, (_, p) => { const v = lookup(p, scopes); return v == null ? '' : String(v); });
  }

  function renderNode(node, scopes, parent) {
    if (node.nodeType === 3) { parent.appendChild(document.createTextNode(interpolate(node.nodeValue, scopes))); return; }
    if (node.nodeType !== 1) return;
    const tag = node.localName;
    if (tag === 'helmet') return;
    if (tag === 'sc-if') {
      const m = WHOLE.exec(node.getAttribute('value') || '');
      if (m && lookup(m[1], scopes)) node.childNodes.forEach((c) => renderNode(c, scopes, parent));
      return;
    }
    if (tag === 'sc-for') {
      const m = WHOLE.exec(node.getAttribute('list') || '');
      const list = (m && lookup(m[1], scopes)) || [];
      const as = node.getAttribute('as') || 'item';
      list.forEach((item, i) => {
        const inner = [{ [as]: item, $index: i }].concat(scopes);
        node.childNodes.forEach((c) => renderNode(c, inner, parent));
      });
      return;
    }
    const el = node.namespaceURI === SVG_NS
      ? document.createElementNS(SVG_NS, node.localName)
      : document.createElement(tag);
    for (const attr of Array.from(node.attributes)) {
      const name = attr.name;
      if (name.startsWith('hint-')) continue;
      const whole = WHOLE.exec(attr.value);
      if (!whole) { el.setAttribute(name, interpolate(attr.value, scopes)); continue; }
      const v = lookup(whole[1], scopes);
      if (name.startsWith('on') && typeof v === 'function') { el.addEventListener(name.slice(2).toLowerCase(), v); continue; }
      if (name.startsWith('aria-')) { el.setAttribute(name, String(!!v === v ? v : (v == null ? '' : v))); continue; }
      if (v === false || v == null) continue;
      if (v === true) { el.setAttribute(name, ''); } else { el.setAttribute(name, String(v)); }
      if (name === 'checked') el.checked = true;
    }
    node.childNodes.forEach((c) => renderNode(c, scopes, el));
    parent.appendChild(el);
  }

  window.mountDC = function (templateEl, logicSource, mountEl) {
    const tpl = templateEl.content;
    const helmet = tpl.querySelector('helmet');
    if (helmet) Array.from(helmet.children).forEach((c) => document.head.appendChild(c.cloneNode(true)));
    const Component = new Function('DCLogic', logicSource + '\nreturn Component;')(DCLogic);
    const props = JSON.parse(templateEl.getAttribute('data-props') || '{}');
    const defaults = {};
    Object.keys(props).forEach((k) => { if (k[0] !== '$' && 'default' in props[k]) defaults[k] = props[k].default; });
    const inst = new Component(defaults);
    inst.__render = function () {
      const vals = inst.renderVals();
      const frag = document.createDocumentFragment();
      tpl.childNodes.forEach((c) => renderNode(c, [vals], frag));
      mountEl.replaceChildren(frag);
    };
    inst.__render();
    if (inst.componentDidMount) inst.componentDidMount();
    window.addEventListener('beforeunload', () => inst.componentWillUnmount && inst.componentWillUnmount());
    return inst;
  };
})();
