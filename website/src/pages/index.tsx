import Layout from '@theme/Layout';
import Link from '@docusaurus/Link';
import Translate, {translate} from '@docusaurus/Translate';
import styles from './index.module.css';

type Feature = {
  icon: string;
  titleId: string;
  titleDefault: string;
  descId: string;
  descDefault: string;
};

const features: Feature[] = [
  {
    icon: '\u{1F50D}', // magnifying glass
    titleId: 'home.feature.analyze.title',
    titleDefault: 'Analyze',
    descId: 'home.feature.analyze.desc',
    descDefault:
      'Parse Rust, Go, Python, TypeScript and more. Build call graphs, module trees, and dependency edges with tree-sitter precision.',
  },
  {
    icon: '\u{1F578}\u{FE0F}', // spider web (graph)
    titleId: 'home.feature.visualise.title',
    titleDefault: 'Visualise',
    descId: 'home.feature.visualise.desc',
    descDefault:
      'Interactive dashboard renders code graphs, hot paths, and architectural layers. Zoom from package to function in one click.',
  },
  {
    icon: '\u{1F4AC}', // speech balloon
    titleId: 'home.feature.explain.title',
    titleDefault: 'Explain',
    descId: 'home.feature.explain.desc',
    descDefault:
      'Embed code chunks, query semantically, and get AI-grounded explanations of how any subsystem actually works.',
  },
];

export default function Home(): JSX.Element {
  const heroTagline = translate({
    id: 'home.hero.tagline',
    message:
      'Rust-native codebase understanding. Analyze, visualise, and explain any project — from a single crate to a multi-language monorepo.',
  });
  const heroBadge = translate({
    id: 'home.hero.badge',
    message: 'early access',
  });
  const ctaPrimary = translate({
    id: 'home.cta.primary',
    message: 'Get Started',
  });
  const ctaGitHub = translate({
    id: 'home.cta.github',
    message: 'GitHub',
  });
  const sectionTitle = translate({
    id: 'home.features.title',
    message: '// What it does',
  });
  const ctaSectionTitle = translate({
    id: 'home.cta.section.title',
    message: 'READ THE GRAPH',
  });
  const ctaSectionText = translate({
    id: 'home.cta.section.text',
    message: 'Stop guessing. Start understanding the codebase you actually have.',
  });
  const ctaSectionButton = translate({
    id: 'home.cta.section.button',
    message: 'Read the Docs',
  });
  const layoutDescription = translate({
    id: 'home.meta.description',
    message:
      'Rust-native codebase understanding tool. Analyze, visualise, explain any project.',
  });

  return (
    <Layout title="understandable" description={layoutDescription}>
      {/* Hero */}
      <section className={styles.hero}>
        <div className={styles.heroBadge}>{heroBadge}</div>
        <div className={styles.heroLogo}>
          understand<span className={styles.heroLogoAccent}>able</span>
        </div>
        <p className={styles.heroTagline}>{heroTagline}</p>
        <div className={styles.heroButtons}>
          <Link className={styles.btnPrimary} to="/docs/getting-started/install">
            {ctaPrimary}
          </Link>
          <Link className={styles.btnSecondary} href="https://github.com/yaroher/understandable">
            {ctaGitHub} &rarr;
          </Link>
        </div>
        <div className={styles.installLine}>
          <span className={styles.installLinePrompt}>$</span> cargo install understandable
        </div>

        <div className={styles.codePreview}>
          <div className={styles.codeHeader}>
            <div className={styles.codeDotR} />
            <div className={styles.codeDotY} />
            <div className={styles.codeDotG} />
            shell
          </div>
          <div className={styles.codeBody}>
{`# scan a project, build the graph, open dashboard
understandable analyze ./my-project
understandable dashboard --open`}
          </div>
        </div>
      </section>

      {/* Features */}
      <section className={styles.features}>
        <div className={styles.featuresTitle}>{sectionTitle}</div>
        <div className={styles.featuresGrid}>
          {features.map((f) => (
            <div key={f.titleId} className={styles.feature}>
              <div className={styles.featureIcon}>{f.icon}</div>
              <div className={styles.featureTitle}>
                <Translate id={f.titleId} description="Landing feature card title">
                  {f.titleDefault}
                </Translate>
              </div>
              <p className={styles.featureDesc}>
                <Translate id={f.descId} description="Landing feature card description">
                  {f.descDefault}
                </Translate>
              </p>
            </div>
          ))}
        </div>
      </section>

      {/* CTA */}
      <section className={styles.cta}>
        <h2 className={styles.ctaTitle}>{ctaSectionTitle}</h2>
        <p className={styles.ctaText}>{ctaSectionText}</p>
        <Link className={styles.btnPrimary} to="/docs/">
          {ctaSectionButton}
        </Link>
      </section>
    </Layout>
  );
}
