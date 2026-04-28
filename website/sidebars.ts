import type {SidebarsConfig} from '@docusaurus/plugin-content-docs';

const sidebars: SidebarsConfig = {
  docs: [
    'intro',
    {
      type: 'category',
      label: 'Getting Started',
      collapsed: false,
      items: ['getting-started/install', 'getting-started/first-graph'],
    },
    {
      type: 'category',
      label: 'CLI Reference',
      items: [
        'cli/analyze',
        'cli/dashboard',
        'cli/embed',
        'cli/init',
      ],
    },
    'architecture',
  ],
};

export default sidebars;
