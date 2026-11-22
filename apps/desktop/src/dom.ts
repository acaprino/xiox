import { keyCaps as packageKeyCaps } from '@acaprino/imago/ui';

// A small element builder: the views build their DOM with it, never with innerHTML, so text that
// comes from a path or a project name is always text.

type Child = Node | string | number | null | undefined | false;
type Attrs = Record<string, string | number | boolean | null | undefined | EventListener>;

export function h<K extends keyof HTMLElementTagNameMap>(tag: K, attrs: Attrs = {}, ...children: Child[]): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  for (const [name, value] of Object.entries(attrs)) {
    if (value === null || value === undefined || value === false) continue;
    if (typeof value === 'function') node.addEventListener(name.replace(/^on/, '').toLowerCase(), value);
    else if (name === 'class') node.className = String(value);
    else if (value === true) node.setAttribute(name, '');
    else node.setAttribute(name, String(value));
  }
  append(node, children);
  return node;
}

export function append(node: Node, children: Child[]): void {
  for (const child of children) {
    if (child === null || child === undefined || child === false) continue;
    node.appendChild(typeof child === 'object' ? child : document.createTextNode(String(child)));
  }
}

/** Replace everything inside `node`. */
export function fill(node: Element, ...children: Child[]): void {
  node.replaceChildren();
  append(node, children);
}

export function button(label: Child | Child[], attrs: Attrs = {}): HTMLButtonElement {
  return h('button', { type: 'button', ...attrs }, ...(Array.isArray(label) ? label : [label]));
}

/** The key caps of a binding, as Imago draws them everywhere (the footer, the menus, the palette): `a` is `A`, `Enter` the return symbol, `Esc` itself. */
export function keyCaps(binding: string): HTMLElement {
  return packageKeyCaps(document, binding);
}

/**
 * The nodes of a string the package wrote (`noteHtml`, `chipHtml` ...): the one way this app turns a package string into
 * elements. Only the package's own markup goes in, with the app's text escaped by the package, never a string of the app's.
 */
export function fromHtml(doc: Document, html: string): DocumentFragment {
  const template = doc.createElement('template');
  template.innerHTML = html;
  return template.content;
}
