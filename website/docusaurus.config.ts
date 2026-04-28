import {themes as prismThemes} from 'prism-react-renderer';
import type {Config} from '@docusaurus/types';
import type * as Preset from '@docusaurus/preset-classic';

const config: Config = {
  title: 'understandable',
  tagline: 'Rust-native codebase understanding — analyze, visualise, explain any project.',
  favicon: 'img/favicon.svg',
  url: 'https://yaroher.github.io',
  baseUrl: '/understandable/',
  organizationName: 'yaroher',
  projectName: 'understandable',
  onBrokenLinks: 'throw',

  markdown: {
    hooks: {
      onBrokenMarkdownLinks: 'warn',
    },
  },

  future: {
    v4: true,
  },

  i18n: {
    defaultLocale: 'en',
    locales: ['en', 'ru'],
    localeConfigs: {
      en: {label: 'English', direction: 'ltr', htmlLang: 'en-US'},
      ru: {label: 'Русский', direction: 'ltr', htmlLang: 'ru-RU'},
    },
  },

  presets: [
    [
      'classic',
      {
        docs: {
          sidebarPath: './sidebars.ts',
          editUrl: 'https://github.com/yaroher/understandable/tree/main/website/',
        },
        blog: false,
        theme: {
          customCss: './src/css/custom.css',
        },
      } satisfies Preset.Options,
    ],
  ],

  themeConfig: {
    colorMode: {
      defaultMode: 'dark',
      disableSwitch: false,
      respectPrefersColorScheme: false,
    },
    navbar: {
      title: 'understandable',
      logo: {
        alt: 'understandable logo',
        src: 'img/logo.svg',
        height: 28,
      },
      style: 'dark',
      items: [
        {
          type: 'docSidebar',
          sidebarId: 'docs',
          position: 'left',
          label: 'Docs',
        },
        {
          href: 'https://github.com/yaroher/understandable',
          label: 'GitHub',
          position: 'right',
        },
        {
          type: 'localeDropdown',
          position: 'right',
        },
      ],
    },
    footer: {
      style: 'dark',
      links: [
        {
          title: 'Docs',
          items: [
            {label: 'Introduction', to: '/docs/'},
            {label: 'Getting Started', to: '/docs/getting-started/install'},
            {label: 'CLI Reference', to: '/docs/cli/analyze'},
            {label: 'Architecture', to: '/docs/architecture'},
          ],
        },
        {
          title: 'Community',
          items: [
            {label: 'GitHub', href: 'https://github.com/yaroher/understandable'},
            {label: 'Issues', href: 'https://github.com/yaroher/understandable/issues'},
          ],
        },
      ],
      copyright: `Copyright © ${new Date().getFullYear()} understandable`,
    },
    prism: {
      theme: prismThemes.vsDark,
      darkTheme: prismThemes.vsDark,
      additionalLanguages: ['rust', 'bash', 'json', 'yaml', 'toml'],
    },
  } satisfies Preset.ThemeConfig,
};

export default config;
