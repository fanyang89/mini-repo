// Full UI flow against a scratch instance: create/edit repo, upload, delete RPM, delete repo.
// Requires: built binary, Chromium headless shell ($CHROME), a sample RPM ($SAMPLE).
const { chromium } = require('playwright')
const fs = require('fs'), net = require('net'), cp = require('child_process')
const repoInList = (t) =>
  [...document.querySelectorAll('aside button')].some((b) => b.textContent.includes(t))

;(async () => {
  const dir = fs.mkdtempSync('/tmp/opencode/mini-spa-')
  const bin = 'target/release/mini-repo'
  cp.execFileSync(bin, ['create', dir])
  const s = net.createServer(); await new Promise(r => s.listen(0, '127.0.0.1', r))
  const port = s.address().port; await new Promise(r => s.close(r))
  const child = cp.spawn(bin, ['serve', dir, '-p', String(port)])
  let b
  let step = 0
  const say = (name) => console.log(`step ${++step}: ${name}`)
  try {
    await new Promise((r, j) => { child.stdout.once('data', r); child.once('error', j) })
    b = await chromium.launch({ executablePath: process.env.CHROME })
    const p = await b.newPage()
    p.on('pageerror', e => console.error('PAGEERROR', e))
    const url = `http://127.0.0.1:${port}`

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
    say('choose file'); await p.locator('input[type=file]').setInputFiles(process.env.SAMPLE)
    say('upload batch'); await p.getByRole('button', { name: 'Upload batch' }).click()
    say('published'); await p.waitForFunction(() =>
      [...document.querySelectorAll('li')].some((li) =>
        li.textContent.includes('published') && li.textContent.includes('rpm-basic')), null, { timeout: 25000 })

    say('back to repositories'); await p.getByRole('button', { name: 'Repositories' }).click()
    say('package listed'); await p.waitForFunction(() => document.querySelector('table')?.textContent.includes('rpm-basic'))
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
    await b?.close(); child.kill('SIGKILL')
    await new Promise(r => child.once('exit', r))
    fs.rmSync(dir, { recursive: true, force: true })
  }
})()
