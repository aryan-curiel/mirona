export type Platform = "macos" | "windows" | "linux";

const ua = navigator.userAgent;
export const detected: Platform = ua.includes("Windows") ? "windows" : ua.includes("Mac") ? "macos" : "linux";

export function applyPlatform(p: Platform) {
  document.documentElement.dataset.platform = p;
}
