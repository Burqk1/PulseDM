import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";

export interface FitOptions {
  width: number;
  min?: number;
  max: number;
}

export function fitWindow(main: HTMLElement, opts: FitOptions) {
  const win = getCurrentWindow();
  const body = main.firstElementChild as HTMLElement;
  let last = 0;
  let shown = false;

  async function fit() {
    const style = getComputedStyle(main);
    const chrome = window.innerHeight - main.clientHeight;
    const wanted = chrome + body.offsetHeight + parseFloat(style.paddingTop) + parseFloat(style.paddingBottom);
    const height = Math.round(Math.min(opts.max, Math.max(opts.min ?? 0, wanted)));
    if (Math.abs(height - last) > 2) {
      last = height;
      await win.setSize(new LogicalSize(opts.width, height));
    }
    if (!shown) {
      shown = true;
      await win.center();
      await win.show();
      await win.setFocus();
    }
  }

  const observer = new ResizeObserver(() => void fit());
  observer.observe(body);
  return {
    update(next: FitOptions) {
      opts = next;
    },
    destroy() {
      observer.disconnect();
    },
  };
}
