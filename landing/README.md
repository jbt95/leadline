# leadline landing page

Marketing site for [leadline](https://github.com/jbt95/leadline). **SolidJS + TypeScript + Tailwind CSS.**

The `web/` package is the React dashboard embedded in `leadline stats`; this folder is separate so the stats bundle and Rust embed paths stay unchanged.

## Commands

```console
cd landing
npm ci
npm run dev      # http://localhost:5173
npm run build    # production build → dist/
npm run preview  # serve dist/
```

Production build runs `tsc --noEmit` then `vite build`.

## Assets

`public/` copies `assets/logo.svg` and the stats screenshots from the repo root for the Surfaces section.
