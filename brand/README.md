# Brand assets

`src/light.png` (black chrome, for light backgrounds), `src/dark.png` (chrome
with edge highlights, for dark backgrounds) and `src/dark-silver.webp` (silver
chrome on solid black, the dark theme's in-app mark) are the NZAP Labs logo
sources. Everything else is generated:

```bash
python brand/build_brand.py brand/src <out-dir>   # needs Pillow and NumPy
npx tauri icon <out-dir>/app-icon-1024.png -o <tmp>  # then copy the files listed in tauri.conf.json
```

| Output                   | Used for                                                    |
| ------------------------ | ----------------------------------------------------------- |
| `app-icon-1024.png`      | source of `src-tauri/icons/*` (dock, taskbar, tray)         |
| `nzap-mark-light-160`    | `src/assets/brand`, the in-app logo on light themes         |
| `nzap-mark-dark-160`     | `src/assets/brand`, the in-app logo on dark themes (silver) |
| `nzap-word-mask.png`     | "NZΛP" lettering, tinted with CSS `mask-image`              |
| `nzap-wordmark-mask.png` | "NZΛP LABS" lettering                                       |
| `favicon-64.png`         | `public/favicon.png`                                        |
| `og-card.png`            | social preview (website)                                    |

`dark-silver.webp` has no transparency; the script recovers it by un-blending
from black (alpha is the brightest channel), so the mark looks exactly as drawn
on black and its glow fades into any dark background.
