import assert from 'node:assert/strict'
import { test } from 'node:test'
import { RSSHUB, viaOurRsshub } from '../src/rsshub.ts'

test('moves rsshub.app and rsshub:// to the site’s instance', () => {
  assert.equal(viaOurRsshub('https://rsshub.app/zhihu/daily'), `${RSSHUB}/zhihu/daily`)
  assert.equal(viaOurRsshub('http://www.rsshub.app/zhihu/hot?limit=5'), `${RSSHUB}/zhihu/hot?limit=5`)
  assert.equal(viaOurRsshub('rsshub://zhihu/daily'), `${RSSHUB}/zhihu/daily`)
  assert.equal(viaOurRsshub('RSSHub://bilibili/user/video/2267573'), `${RSSHUB}/bilibili/user/video/2267573`)
  assert.equal(viaOurRsshub(' https://RSSHub.app '), `${RSSHUB}/`)
})

test('leaves other addresses alone', () => {
  for (const url of [
    'https://blog.example.com/feed.xml',
    'https://myrsshub.app/zhihu/daily',
    'https://rsshub.application.com/zhihu/daily',
    'https://example.com/rsshub.app/feed',
    `${RSSHUB}/zhihu/daily`,
    'file:subscriptions.opml',
  ]) {
    assert.equal(viaOurRsshub(url), url)
  }
})
