// E2E for the `keepStyles` environment-based default under vitest BROWSER
// mode: the transform
// rewrites the component's `styleUrl` to a `?inline` import, vite compiles the
// scss (sass features and all), and Angular JIT applies the resulting CSS
// string — so real computed styles are observable in the page.
import '@angular/compiler';
import { TestBed, getTestBed } from '@angular/core/testing';
import { BrowserTestingModule, platformBrowserTesting } from '@angular/platform-browser/testing';
import { describe, expect, it } from 'vitest';
import { CardComponent } from './fixtures/card.component';

getTestBed().initTestEnvironment(BrowserTestingModule, platformBrowserTesting());

describe('keepStyles in browser mode', () => {
  it('applies the vite-compiled scss to the rendered component', () => {
    const fixture = TestBed.createComponent(CardComponent);
    fixture.detectChanges();

    const card = fixture.nativeElement.querySelector('.card') as HTMLElement;
    const title = fixture.nativeElement.querySelector('.card-title') as HTMLElement;
    // Both rules come from sass constructs ($variables + nesting), so these
    // computed values prove the full chain: ?inline rewrite → vite scss
    // compile → JIT `styles: [...]` → stylesheet applied to the document.
    expect(getComputedStyle(card).padding).toBe('12px');
    expect(getComputedStyle(title).color).toBe('rgb(0, 128, 0)');
  });

  it('keeps the compiled CSS text in the component metadata', () => {
    // The JIT-compiled definition carries the inlined stylesheet — the sass
    // variable must already be resolved (no `$` left) by vite's css pipeline.
    const def = (CardComponent as unknown as { ɵcmp: { styles: string[] } }).ɵcmp;
    const css = def.styles.join('\n');
    expect(css).toContain('padding');
    expect(css).not.toContain('$card-padding');
  });
});
