// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import ShellScript from './ShellScript.vue'

function mount(source: string) {
  return renderWithPlugins(ShellScript, { props: { source } })
}

describe('ShellScript', () => {
  // The bug this component exists for: an inline `<code>` let the browser
  // collapse every newline and run of indentation into a single space, so a
  // multi-line hook command arrived as one unreadable paragraph.
  it('renders the script verbatim, newlines and indentation included', () => {
    const script = 'if true; then\n    echo indented\nfi'
    expect(mount(script).text()).toBe(script)
  })

  it('renders on the shared pre-wrap block rather than an inline element', () => {
    expect(mount('echo hi').find('.detail-pre').exists()).toBe(true)
  })

  it('colours each run by what it is', () => {
    const wrapper = mount("echo 'hi' # note")
    expect(wrapper.find('.sh-command').text()).toBe('echo')
    expect(wrapper.find('.sh-string').text()).toBe("'hi'")
    expect(wrapper.find('.sh-comment').text()).toBe('# note')
  })

  it('renders an empty script without spans', () => {
    const wrapper = mount('')
    expect(wrapper.find('.detail-pre').text()).toBe('')
    expect(wrapper.findAll('span')).toHaveLength(0)
  })

  // A hook command is arbitrary user input that lands in a page: it has to be
  // text content, never markup.
  it('does not interpret markup in the script', () => {
    const wrapper = mount('echo "<img src=x onerror=alert(1)>"')
    expect(wrapper.find('img').exists()).toBe(false)
    expect(wrapper.text()).toContain('<img src=x onerror=alert(1)>')
  })
})
