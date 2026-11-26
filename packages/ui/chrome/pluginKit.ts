// The native-look kit every plugin frame gets (docs/dev/plugins.md Interface): a style element of its own after the
// page's tokens and before the piece's styles, so a piece built from these classes reads as part of Acpira and follows
// the theme, density and skin without copying the page's CSS. Built on the tokens tokensOf passes (panel/PluginFrame.tsx),
// matching ui/Button (two neutral tiers), the chip, ui/Row, ui/Card and the inputs. Class names are a public contract:
// rename one and every plugin using it falls back to plain HTML
export const FRAME_KIT = `
*,*::before,*::after{box-sizing:border-box}
:focus-visible{outline:1px solid var(--focus);outline-offset:-1px}
.a-stack{display:flex;flex-direction:column;gap:var(--gap)}
.a-line{display:flex;align-items:center;gap:var(--gap);min-width:0}
.a-title{font-size:var(--text-2);font-weight:500;color:var(--fg-1);margin:0}
.a-muted{font-size:var(--text-3);color:var(--fg-3)}
.a-mono{font-family:var(--font-mono);font-size:var(--code-font-size)}
.a-truncate{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;min-width:0}
.a-btn{display:inline-flex;align-items:center;gap:6px;height:var(--ctl);padding:0 12px;border:1px solid var(--line);border-radius:var(--r-md);background:transparent;color:var(--fg-2);font:inherit;font-size:var(--text-2);white-space:nowrap;cursor:pointer;transition:background-color .15s,border-color .15s,color .15s}
.a-btn:hover{border-color:var(--line-strong);background:var(--active);color:var(--fg-1)}
.a-btn.primary{border-color:var(--line-strong);background:var(--active);color:var(--fg-strong);font-weight:500}
.a-btn.primary:hover{background:var(--chip-hover)}
.a-btn.icon{width:var(--ctl);padding:0;justify-content:center;border-color:transparent}
.a-btn:disabled{opacity:.5;cursor:default;pointer-events:none}
.a-chip{display:inline-flex;align-items:center;gap:4px;height:var(--ctl-sm);min-width:0;padding:0 6px;border:0;border-radius:var(--r-sm);background:var(--chip);color:var(--fg-2);font:inherit;font-size:var(--text-3);white-space:nowrap;cursor:pointer;transition:background-color .15s,color .15s}
.a-chip:hover{background:var(--chip-hover);color:var(--fg-1)}
.a-chip.on{color:var(--fg-strong);background:var(--active)}
.a-row{display:flex;align-items:center;gap:var(--gap);min-height:var(--row);padding:0 var(--pad);border-radius:var(--r-md);color:var(--fg-2);font-size:var(--text-2)}
.a-row:is(button,a,[tabindex]){border:0;background:transparent;font:inherit;text-align:start;width:100%;cursor:pointer}
.a-row:is(button,a,[tabindex]):hover{background:var(--hover);color:var(--fg-1)}
.a-card{border:1px solid var(--line);border-radius:var(--r-lg);background:var(--bg-1);padding:var(--pad)}
.a-input{height:var(--ctl);width:100%;padding:0 8px;border:1px solid var(--line);border-radius:var(--r-md);background:var(--input-bg,var(--bg-2));color:var(--fg-1);font:inherit;font-size:var(--text-2)}
.a-input:focus{border-color:var(--focus);outline:0}
.a-rule{height:1px;border:0;margin:0;background:var(--line)}
.a-dot{width:6px;height:6px;border-radius:50%;background:var(--fg-3);flex:none}
.a-dot.ok{background:var(--ok)}.a-dot.warn{background:var(--warn)}.a-dot.danger{background:var(--danger)}.a-dot.brand{background:var(--brand)}
`;
