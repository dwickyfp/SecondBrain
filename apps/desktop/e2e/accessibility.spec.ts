import AxeBuilder from '@axe-core/playwright';
import { expect, test } from '@playwright/test';
import { installTransport, openWorkspaceByKeyboard } from './fixture';

test.beforeEach(async ({ page }) => installTransport(page));

async function expectAccessible(page: import('@playwright/test').Page) {
  const result = await new AxeBuilder({ page }).analyze();
  expect(result.violations, result.violations.map((item) => `${item.id}: ${item.help}`).join('\n')).toEqual([]);
}

test('welcome state is accessible and its folder actions follow keyboard order', async ({ page }) => {
  await page.goto('/');
  const welcome = await page.locator('.welcome').boundingBox();
  const story = await page.locator('.welcome-story').boundingBox();
  const actions = await page.locator('.welcome-actions').boundingBox();
  expect(welcome).not.toBeNull();
  expect(story).not.toBeNull();
  expect(actions).not.toBeNull();
  expect(welcome!.width).toBeGreaterThan(900);
  expect(story!.x + story!.width).toBeLessThanOrEqual(actions!.x + 1);
  expect(actions!.width).toBeGreaterThan(430);
  const pathInput = await page.getByLabel('Workspace folder').boundingBox();
  const chooseButton = await page.getByRole('button', { name: 'Choose folder' }).boundingBox();
  expect(pathInput).not.toBeNull();
  expect(chooseButton).not.toBeNull();
  expect(Math.abs(pathInput!.y - chooseButton!.y)).toBeLessThanOrEqual(1);
  expect(Math.abs(pathInput!.height - chooseButton!.height)).toBeLessThanOrEqual(1);
  await expectAccessible(page);
  await page.keyboard.press('Tab');
  const choose = page.getByRole('button', { name: 'Choose folder' });
  await expect(choose).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page.getByLabel('Workspace folder')).toHaveValue('/fixture/vault');

  await page.keyboard.press('Tab');
  await expect(page.getByLabel('Workspace folder')).toBeFocused();
  await page.keyboard.press('Tab');
  await expect(page.getByRole('button', { name: 'Open workspace' })).toBeFocused();
  await page.keyboard.press('Tab');
  const importButton = page.getByRole('button', { name: 'Review Obsidian import' });
  await expect(importButton).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('heading', { name: 'Import review' })).toBeVisible();
  await expectAccessible(page);
});

test('keyboard workspace, palette containment and restored focus', async ({ page }) => {
  await openWorkspaceByKeyboard(page);
  await expectAccessible(page);
  const search = page.getByLabel('Search workspace');
  await search.focus();
  await page.keyboard.press('Meta+k');
  const palette = page.getByRole('dialog', { name: 'Command palette' });
  await expect(palette).toBeVisible();
  await expect(page.getByRole('combobox', { name: 'Find a command' })).toBeFocused();
  for (let index = 0; index < 20; index += 1) {
    await page.keyboard.press('Tab');
    await expect(palette.locator(':focus')).toHaveCount(1);
  }
  await page.keyboard.press('Escape');
  await expect(search).toBeFocused();
});

test('tabs, splits and active panes are keyboard operable without a trap', async ({ page }) => {
  await openWorkspaceByKeyboard(page);
  await page.getByRole('button', { name: 'Linked note' }).click();
  const linkedTab = page.getByRole('tab', { name: 'Linked note' });
  await linkedTab.focus();
  await linkedTab.press('ArrowLeft');
  await expect(page.getByRole('tab', { name: 'Welcome' })).toHaveAttribute('aria-selected', 'true');
  await page.getByRole('button', { name: 'Split', exact: true }).first().focus();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('region', { name: 'Secondary pane', exact: true })).toBeVisible();
  await page.keyboard.press('Control+1');
  await expect(page.locator('[data-pane-content="primary"]')).toBeFocused();
  await page.keyboard.press('F6');
  await expect(page.locator('[data-pane-content="secondary"]')).toBeFocused();
  await page.keyboard.press('Tab');
  await expect(page.locator('body :focus')).toHaveCount(1);
  await expectAccessible(page);
});

test('source edit preview apply, graph navigation and recovery remain accessible', async ({ page }) => {
  await openWorkspaceByKeyboard(page);
  await page.getByRole('button', { name: 'Source', exact: true }).click();
  const editor = page.getByRole('textbox', { name: 'Welcome Markdown source' });
  await editor.click();
  await page.keyboard.press('Control+End');
  await page.keyboard.type('\nKeyboard edit.');
  await page.getByRole('button', { name: 'Preview', exact: true }).click();
  await page.getByRole('button', { name: 'Apply', exact: true }).click();
  await page.getByRole('button', { name: 'Graph' }).click();
  await page.getByLabel('Workspace graph table').getByRole('button', { name: 'Linked note' }).click();
  await expect(page.getByRole('heading', { name: 'Linked note', level: 1 }).first()).toBeVisible();
  await page.getByRole('button', { name: /healthy|checking/i }).click();
  await page.getByRole('button', { name: 'Run safe recovery' }).click();
  await expect(page.getByText('SB-JOURNAL-QUARANTINED: quarantined')).toBeVisible();
  await expect(page.getByText(/Preserved at .*quarantine\.bin/)).toBeVisible();
  await expectAccessible(page);
});

test('import review and errors are announced', async ({ page }) => {
  await page.goto('/');
  await page.getByLabel('Workspace folder').fill('/fixture/import');
  await page.getByRole('button', { name: 'Review Obsidian import' }).click();
  await expect(page.getByRole('heading', { name: 'Import review' })).toBeVisible();
  await expectAccessible(page);
});
