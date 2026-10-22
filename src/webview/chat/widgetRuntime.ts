// Tailwind's browser build as text, inlined into every widget frame (`widgetDoc.ts`): the frame has no network to load it
// from. Its own chunk, fetched by the first widget, not with the page
export { default as tailwind } from '@tailwindcss/browser?raw';
