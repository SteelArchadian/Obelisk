/**
 * Icons: smooth when shrunk (big logos), sharp pixels when enlarged (16 px pixel-art icons).
 * Use on an <img>: `use:crisp`; it adds the "px" class when the image is drawn larger than it is.
 */
export function crisp(img: HTMLImageElement) {
  const set = () => img.classList.toggle("px", img.naturalWidth > 0 && img.naturalWidth < img.clientWidth * devicePixelRatio);
  img.addEventListener("load", set);
  if (img.complete) set();
  return { destroy: () => img.removeEventListener("load", set) };
}
