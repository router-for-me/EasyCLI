import { useEffect, useState } from 'react';

const OVERLAY_SELECTOR = 'dialog[open], [role="dialog"], [role="alertdialog"], [role="listbox"], [role="menu"]';
const OBSERVED_ATTRIBUTES = ['open', 'hidden', 'class', 'style', 'role', 'aria-hidden'];

const isVisible = (element: Element): boolean => {
  if (element.getClientRects().length === 0) return false;
  const style = window.getComputedStyle(element);
  return style.visibility !== 'hidden' && style.display !== 'none';
};

const isOverlayOpen = (): boolean =>
  Array.from(document.querySelectorAll(OVERLAY_SELECTOR)).some(isVisible);

export function useOverlayOpen(): boolean {
  const [open, setOpen] = useState(isOverlayOpen);

  useEffect(() => {
    let frame = 0;
    const update = () => {
      frame = 0;
      setOpen(isOverlayOpen());
    };
    const observer = new MutationObserver(() => {
      if (!frame) frame = window.requestAnimationFrame(update);
    });
    observer.observe(document.body, { subtree: true, childList: true, attributes: true, attributeFilter: OBSERVED_ATTRIBUTES });
    update();
    return () => {
      observer.disconnect();
      if (frame) window.cancelAnimationFrame(frame);
    };
  }, []);

  return open;
}
