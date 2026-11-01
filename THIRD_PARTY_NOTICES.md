# Third-party notices

The visual effects under `packages/ui/effects/` are ports of four libraries by Jakub Antalik ([Libraries.dev](https://libraries.dev)). Their geometry, tuning constants, palettes and shader logic follow the originals; the rendering code was rewritten for Acpira.

| Acpira file | Ported from |
|---|---|
| `packages/ui/effects/orbEngine.ts` | [thinking-orbs](https://orbs.jakubantalik.com) 0.3.1 |
| `packages/ui/effects/Beam.tsx`, `packages/ui/styles/beam.css` | [border-beam](https://beam.jakubantalik.com) 1.3.0 |
| `packages/ui/effects/metalPaint.ts` | [metal-fx](https://metal.jakubantalik.com) 1.0.4 |
| `packages/ui/effects/mosaicGL.ts` | [img-fx](https://image.jakubantalik.com) 0.5.1 |

All four are released under the same license:

```
MIT License

Copyright (c) 2026 Jakub Antalik

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
