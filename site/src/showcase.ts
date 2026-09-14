import { ansiToHtml } from '@casoon/pages-theme/ansi';
import type { ShowcaseExample } from '@casoon/pages-theme/showcase';

// Real `cargo broom` output over sample projects, captured by examples/generate.sh.
// Cleanup commands run with --dry-run; the script checks that nothing was removed.
const files = import.meta.glob<string>('../../examples/*.txt', {
  query: '?raw',
  import: 'default',
  eager: true,
});

const examples_ = [
  {
    slug: 'dry-run',
    title: 'Dry run over a directory tree',
    file: 'dry-run.txt',
    command: 'cargo broom --dry-run ~/code',
    tags: ['dry-run', 'level-a', 'multi-project'],
    description:
      'Two projects not built for months are proposed for a full target removal. A fresh build, a workspace and an old but shared target are left alone.',
  },
  {
    slug: 'keep-size',
    title: 'Minimum size',
    file: 'keep-size.txt',
    command: 'cargo broom --dry-run --keep-size 1500KB ~/code',
    tags: ['dry-run', 'policy'],
    description: 'With a size threshold, the smaller of the two cold targets stays.',
  },
  {
    slug: 'selective-cleanup',
    title: 'Selective cleanup',
    file: 'dry-run-fine.txt',
    command: 'cargo broom --dry-run --clean-incremental --clean-doc ~/code',
    tags: ['dry-run', 'level-b'],
    description:
      'Cold targets go completely; active ones, including the shared target, only lose incremental caches and generated docs.',
  },
  {
    slug: 'single-project',
    title: 'One project',
    file: 'project.txt',
    command: 'cargo broom project ~/code/weather-api --dry-run',
    tags: ['dry-run', 'project'],
    description: 'The same policy and safety gates, applied to a single project.',
  },
  {
    slug: 'inspect',
    title: 'Inspect',
    file: 'inspect.txt',
    command: 'cargo broom inspect ~/code',
    tags: ['read-only', 'multi-project'],
    description:
      'Every discovered target with its size. The workspace appears once; the two plugins share one overridden target.',
  },
  {
    slug: 'budget',
    title: 'Budget',
    file: 'budget.txt',
    command: 'cargo broom budget ~/code --limit 5MB',
    tags: ['read-only', 'multi-project'],
    description:
      'Total target usage against one budget for the whole tree. Over budget, it names the largest targets; --details lists all of them.',
  },
  {
    slug: 'dry-run-json',
    title: 'JSON report',
    file: 'dry-run-json.txt',
    command: 'cargo broom --dry-run --format json ~/code',
    tags: ['dry-run', 'json'],
    description: 'The dry run as JSON on stdout, for scripts and logged scheduled runs.',
  },
  {
    slug: 'refused',
    title: 'No confirmation, no cleanup',
    file: 'refused.txt',
    command: 'cargo broom ~/code',
    tags: ['safety', 'error'],
    description:
      'Without --dry-run, --interactive or --yes, cargo-broom refuses before scanning and exits with status 1.',
  },
];

export const examples: ShowcaseExample[] = examples_.map(({ file, command, ...meta }) => {
  const source = files[`../../examples/${file}`] ?? '';
  return {
    ...meta,
    file: `examples/${file}`,
    input: { code: command, lang: 'shell' },
    output: { html: ansiToHtml(source), kind: 'terminal' },
  };
});
