/** Ctrl + mouse wheel over an element calls `step` with +1 (wheel up, bigger) or -1 (down, smaller), for sizing
 *  tiles and rows (user, 2026-11-06: easier to see). A plain wheel still scrolls. Svelte's onwheel is passive, so
 *  it couldn't stop the scroll; this listener isn't. */
export function ctrlWheel(node: HTMLElement, step: (dir: 1 | -1) => void) {
  let fn = step;
  let acc = 0; // touchpads send many small deltas: one step per notch's worth
  const on = (e: WheelEvent) => {
    if (!e.ctrlKey) return;
    e.preventDefault();
    acc += e.deltaY;
    if (Math.abs(acc) < 50) return;
    fn(acc < 0 ? 1 : -1);
    acc = 0;
  };
  node.addEventListener("wheel", on, { passive: false });
  return {
    update(next: (dir: 1 | -1) => void) { fn = next; },
    destroy() { node.removeEventListener("wheel", on); },
  };
}
