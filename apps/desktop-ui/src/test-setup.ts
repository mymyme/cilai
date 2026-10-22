// jsdom does not implement the browser APIs the shell relies on for layout and identifiers.
// These stubs only need to keep effects from throwing; no test asserts on their behaviour.
class ResizeObserverStub implements ResizeObserver {
  observe() { /* layout measurement is not exercised in jsdom */ }
  unobserve() { /* layout measurement is not exercised in jsdom */ }
  disconnect() { /* layout measurement is not exercised in jsdom */ }
}

globalThis.ResizeObserver = ResizeObserverStub;
window.scrollTo = () => { /* jsdom has no viewport to pin */ };
HTMLCanvasElement.prototype.getContext = (() => ({
  drawImage() { /* frame drawing is asserted with dedicated mocks */ },
  putImageData() { /* bitmap drawing is asserted with dedicated mocks */ },
})) as never;

if (!globalThis.crypto?.randomUUID) {
  Object.defineProperty(globalThis.crypto, "randomUUID", {
    value: () => "00000000-0000-4000-8000-000000000000",
    configurable: true,
  });
}

// jsdom implements no layout, so it ships no scrollIntoView. Components that keep the active
// option in view call it during a normal open; without this they would throw only under test.
if (!Element.prototype.scrollIntoView) Element.prototype.scrollIntoView = () => {};

/*
 * jsdom 也缺 CSS.escape 与 scrollIntoView，组件按消息标识取元素或定位菜单时会用到。
 * 同样只补在测试环境，产品代码不为 jsdom 让步。
 */
if (typeof globalThis.CSS?.escape !== "function") {
  const escape = (value: string) => String(value).replace(/["'\\\]\[]/g, "\\$&");
  if (globalThis.CSS) Object.defineProperty(globalThis.CSS, "escape", { configurable: true, value: escape, writable: true });
  else Object.defineProperty(globalThis, "CSS", { configurable: true, value: { escape }, writable: true });
}

if (typeof Element.prototype.scrollIntoView !== "function") {
  Object.defineProperty(Element.prototype, "scrollIntoView", {
    configurable: true,
    value: () => {},
    writable: true,
  });
}
