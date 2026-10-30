// The runtime the page sends into a component piece's frame (packages/ui's build writes it to
// dist/webview/plugin-ui.js; pluginSlots.ts fetches it). It publishes React and the kit as `globalThis.__acpiraLib`,
// where the engine's module loader (rust/.../plugins/build.rs) finds every import a piece leaves to the page. Each entry
// is a module namespace with its `default`, as `import React from 'react'` expects

import * as React from 'react';
import * as JsxRuntime from 'react/jsx-runtime';
import * as ReactDom from 'react-dom';
import * as ReactDomClient from 'react-dom/client';
import * as Kit from './index';

const namespace = <T extends object>(ns: T) => Object.freeze({ ...ns, default: (ns as { default?: unknown }).default ?? ns });

(globalThis as { __acpiraLib?: unknown }).__acpiraLib = Object.freeze({
  react: namespace(React),
  'react/jsx-runtime': namespace(JsxRuntime),
  // Production builds lower JSX the same way; a piece compiled for development still finds its runtime
  'react/jsx-dev-runtime': namespace({ ...JsxRuntime, jsxDEV: JsxRuntime.jsx }),
  'react-dom': namespace(ReactDom),
  'react-dom/client': namespace(ReactDomClient),
  '@acpira/plugin-ui': namespace(Kit),
});
