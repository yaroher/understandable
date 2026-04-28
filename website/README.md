# understandable — website

Docusaurus 3 site with bilingual (en + ru) documentation.

## Develop

```bash
npm install
npm start            # English
npm run start:ru     # Russian
```

## Build

```bash
npm run build
```

## i18n

- Default locale: `en`
- Other locales: `ru`
- UI strings: `i18n/<locale>/code.json`
- Theme strings (navbar, footer): `i18n/<locale>/docusaurus-theme-classic/*.json`
- Doc translations: `i18n/<locale>/docusaurus-plugin-content-docs/current/*.md`

Regenerate translation source files when you add new `<Translate>` calls or
new docs:

```bash
npm run write-translations -- --locale ru
```
