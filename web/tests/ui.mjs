// tests/smoke.py --ui supplies a scratch server and a generated noarch RPM.
import { chromium } from 'playwright'
import { basename } from 'node:path'

const [url, sample, authorization] = process.argv.slice(2)
const filename = basename(sample)
const repoInList = (t) =>
  [...document.querySelectorAll('aside button')].some((b) => b.textContent.includes(t))

async function testUI() {
  const b = await chromium.launch()
  let step = 0
  const say = (name) => console.log(`step ${++step}: ${name}`)
  try {
    const p = await b.newPage({ extraHTTPHeaders: { Authorization: authorization } })
    p.on('pageerror', e => console.error('PAGEERROR', e))

    say('open'); await p.goto(url + '/mini-repo/')
    say('empty state'); await p.getByRole('button', { name: 'Create repository' }).first().click()
    for (const [key, v] of Object.entries({ repo: 'os-test', name: 'EL test' })) {
      say(`fill ${key}`); await p.locator(`[id=${key}]`).fill(v)
    }
    say('pick arch'); await p.locator('[id=arch]').click(); await p.getByRole('option', { name: 'x86_64' }).click()
    say('submit create'); await p.getByRole('button', { name: 'Create', exact: true }).click()
    say('repo listed'); await p.waitForFunction(repoInList, 'os-test')

    say('settings tab'); await p.getByRole('tab', { name: 'Settings' }).click()
    say('edit name'); await p.locator('#edit-name').fill('EL ten')
    say('save'); await p.getByRole('button', { name: 'Save changes' }).click()
    say('name updated'); await p.waitForFunction(() => document.body.textContent.includes('EL ten ·'))

    say('open upload'); await p.getByRole('button', { name: 'Upload', exact: true }).click()
    say('pick repo'); await p.locator('button[role="combobox"]').click(); await p.getByRole('option').click()
    say('choose file'); await p.locator('input[type=file]').setInputFiles(sample)
    say('upload batch'); await p.getByRole('button', { name: 'Upload batch' }).click()
    say('published'); await p.waitForFunction((file) =>
      [...document.querySelectorAll('li')].some((li) =>
        li.textContent.includes('published') && li.textContent.includes(file)), filename, { timeout: 25000 })

    say('back to repositories'); await p.getByRole('button', { name: 'Repositories' }).click()
    say('package listed'); await p.locator(`table a[href$="/${filename}"]`).waitFor()
    say('delete rpm'); await p.getByRole('button', { name: 'Delete', exact: true }).click()
    say('confirm'); await p.getByRole('button', { name: 'Delete', exact: true }).click()
    say('table empty'); await p.waitForFunction(() => !document.querySelector('table tbody tr'))

    say('settings tab'); await p.getByRole('tab', { name: 'Settings' }).click()
    say('delete repo'); await p.getByRole('button', { name: 'Delete repository' }).click()
    say('type id'); await p.getByRole('dialog').getByPlaceholder('os-test').fill('os-test')
    say('confirm'); await p.getByRole('button', { name: 'Delete', exact: true }).click()
    say('empty state back'); await p.waitForFunction(() => document.body.textContent.includes('No repositories yet'))
    console.log('Chromium OK: redesigned SPA create/edit repo, upload/delete RPM, delete repo')
  } catch (e) {
    console.error(`failed at step ${step}`); console.error(e); process.exitCode = 1
  } finally {
    await b.close()
  }
}

await testUI()
