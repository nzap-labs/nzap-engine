# Brand assets

`src/light.png` (black chrome, for light backgrounds) and `src/dark.png`
(chrome with edge highlights, for dark backgrounds) are the NZAP Labs logo
sources. Everything else is generated:

```bash
python brand/build_brand.py brand/src <out-dir>   # needs Pillow
npx tauri icon <out-dir>/app-icon-1024.png -o <tmp>  # then copy the files listed in tauri.conf.json
```

| Output                       | Used for                                            |
| ---------------------------- | --------------------------------------------------- |
| `app-icon-1024.png`          | source of `src-tauri/icons/*` (dock, taskbar, tray) |
| `nzap-mark-{light,dark}-160` | `src/assets/brand`, the in-app logo per theme       |
| `nzap-word-mask.png`         | "NZΛP" lettering, tinted with CSS `mask-image`      |
| `nzap-wordmark-mask.png`     | "NZΛP LABS" lettering                               |
| `favicon-64.png`             | `public/favicon.png`                                |
| `og-card.png`                | social preview (website)                            |
