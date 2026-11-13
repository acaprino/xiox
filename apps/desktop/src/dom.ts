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

export function kbd(text: string): HTMLElement {
  return h('kbd', {}, text);
}
