// @ts-check
import casoonPages from '@casoon/pages-theme';
import { defineConfig } from 'astro/config';

// Project page: https://casoon.github.io/broom/ — `base` is the GitHub Pages path.
export default defineConfig({
  site: 'https://casoon.github.io/broom',
  base: '/broom/',
  integrations: [
    casoonPages({
      name: 'cargo-broom',
      description: 'Conservative Cargo build-artifact cleaner for one or many Rust projects.',
      repo: 'casoon/broom',
      version: '0.1.0',
      license: 'MIT',
      packages: [
        { label: 'crates.io', href: 'https://crates.io/crates/cargo-broom' },
        { label: 'docs.rs', href: 'https://docs.rs/cargo-broom' },
      ],
      docsGroups: {
        'getting-started': 'Getting started',
        guides: 'Guides',
        reference: 'Reference',
      },
    }),
  ],
});
