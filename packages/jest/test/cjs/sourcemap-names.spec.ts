import { createTransformer } from '../../dist/index.js';

// tsc's transpileModule emits sourcemaps with NO `names`; jest remaps stack
// frame labels through the names array, so a map that carries them relabels
// frames (Object.<anonymous> → Object.<calleeName>) and breaks code that
// fingerprints Error.stack. The transformer must strip them for parity —
// names emptied AND every mapping segment truncated to its 4 positional
// fields, positions untouched.
describe('@oxc-angular-testing/jest — sourcemap names parity', () => {
  it('emits maps with no names and 4-field segments', () => {
    const t = createTransformer({ module: 'commonjs', transform: { jitTransforms: false } });
    const out = t.process(
      "import { helper } from './h';\nexport const x = helper();\n",
      '/p/s.ts',
      {}
    ) as { map?: { names: string[]; mappings: string } };
    expect(out.map).toBeDefined();
    expect(out.map!.names).toEqual([]);
    expect(out.map!.mappings.length).toBeGreaterThan(0);
    // No segment may retain a 5th VLQ value: decode value counts per segment.
    const B64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
    for (const line of out.map!.mappings.split(';')) {
      for (const seg of line.split(',')) {
        let values = 0;
        for (const ch of seg) {
          if ((B64.indexOf(ch) & 32) === 0) values++;
        }
        expect(values).toBeLessThanOrEqual(4);
      }
    }
  });
});
