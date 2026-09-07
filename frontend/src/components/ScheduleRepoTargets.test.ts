// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import ScheduleRepoTargets from './ScheduleRepoTargets.vue'
import type { ScheduleRepoTarget } from '../api/schedules'
import type { Repo } from '../types/repo'

const REPOS = [
  { id: 20, name: 'nas-local', ssh_user: 'borg', ssh_host: 'nas.lan', repo_path: '/srv/borg/main' },
  {
    id: 21,
    name: 'offsite',
    ssh_user: 'u1',
    ssh_host: 'offsite.example',
    repo_path: '/backups',
  },
  {
    id: 22,
    name: 'nas-archive',
    ssh_user: 'borg',
    ssh_host: 'nas.lan',
    repo_path: '/srv/borg/archive',
  },
] as unknown as Repo[]

function mount(modelValue: ScheduleRepoTarget[], props: Record<string, unknown> = {}) {
  return renderWithPlugins(ScheduleRepoTargets, {
    props: { repos: REPOS, modelValue, ...props },
  })
}

function lastModel(wrapper: ReturnType<typeof mount>): ScheduleRepoTarget[] {
  return wrapper.emitted('update:modelValue')?.at(-1)?.[0] as ScheduleRepoTarget[]
}

describe('ScheduleRepoTargets', () => {
  it('lists each target with its address and required state', () => {
    const wrapper = mount([
      { repo_id: 20, required: true },
      { repo_id: 21, required: false },
    ])

    expect(wrapper.findAll('.order-item')).toHaveLength(2)
    expect(wrapper.text()).toContain('borg@nas.lan:/srv/borg/main')
    expect(wrapper.findAll('.badge').map((b) => b.text())).toEqual(['Required', 'Best effort'])
    expect(wrapper.text()).toContain('1 of 2 required')
  })

  it('adds the first repository that is not already a target, as best effort', async () => {
    const wrapper = mount([{ repo_id: 20, required: true }])
    await wrapper
      .findAll('button')
      .find((b) => b.text() === 'Add repository')!
      .trigger('click')

    expect(lastModel(wrapper)).toEqual([
      { repo_id: 20, required: true },
      { repo_id: 21, required: false },
    ])
  })

  it('offers no more repositories once every one is a target', () => {
    const wrapper = mount(REPOS.map((r) => ({ repo_id: r.id, required: true })))
    const add = wrapper.findAll('button').find((b) => b.text().includes('already a target'))
    expect(add?.attributes('disabled')).toBeDefined()
  })

  it('disables a repository already used by another target', () => {
    const wrapper = mount([
      { repo_id: 20, required: true },
      { repo_id: 21, required: false },
    ])
    const firstSelect = wrapper.findAll('select')[0]
    const disabled = firstSelect
      .findAll('option')
      .filter((o) => o.attributes('disabled') !== undefined)
      .map((o) => o.attributes('value'))
    expect(disabled).toEqual(['21'])
  })

  it('reorders targets', async () => {
    const wrapper = mount([
      { repo_id: 20, required: true },
      { repo_id: 21, required: false },
    ])
    await wrapper.findAll('.order-btn[title="Move repository down"]')[0].trigger('click')

    expect(lastModel(wrapper)).toEqual([
      { repo_id: 21, required: false },
      { repo_id: 20, required: true },
    ])
  })

  it('moves a target back up the list', async () => {
    const wrapper = mount([
      { repo_id: 20, required: true },
      { repo_id: 21, required: false },
    ])
    await wrapper.findAll('.order-btn[title="Move repository up"]')[1].trigger('click')

    expect(lastModel(wrapper)).toEqual([
      { repo_id: 21, required: false },
      { repo_id: 20, required: true },
    ])
  })

  it('leaves the list alone at either end', async () => {
    const wrapper = mount([
      { repo_id: 20, required: true },
      { repo_id: 21, required: false },
    ])
    expect(
      wrapper.find('.order-btn[title="Move repository up"]').attributes('disabled'),
    ).toBeDefined()
    expect(
      wrapper.findAll('.order-btn[title="Move repository down"]')[1].attributes('disabled'),
    ).toBeDefined()
  })

  it('removes a target but never the last one', async () => {
    const wrapper = mount([
      { repo_id: 20, required: true },
      { repo_id: 21, required: false },
    ])
    await wrapper.findAll('.order-btn[title="Remove repository"]')[1].trigger('click')
    expect(lastModel(wrapper)).toEqual([{ repo_id: 20, required: true }])

    const single = mount([{ repo_id: 20, required: true }])
    expect(
      single.find('.order-btn[title="Remove repository"]').attributes('disabled'),
    ).toBeDefined()
  })

  it('flags two targets that live on the same storage host', () => {
    const wrapper = mount([
      { repo_id: 20, required: true },
      { repo_id: 22, required: false },
    ])
    expect(wrapper.findAll('.repo-target--warned')).toHaveLength(2)
    expect(wrapper.text()).toContain('Shares a storage host with another target')
  })

  it('does not flag targets on different hosts', () => {
    const wrapper = mount([
      { repo_id: 20, required: true },
      { repo_id: 21, required: false },
    ])
    expect(wrapper.findAll('.repo-target--warned')).toHaveLength(0)
  })

  it('switches a target between required and best effort', async () => {
    const wrapper = mount([
      { repo_id: 20, required: true },
      { repo_id: 21, required: false },
    ])
    await wrapper.findAllComponents({ name: 'ToggleSwitch' })[1].vm.$emit('update:modelValue', true)

    expect(lastModel(wrapper)).toEqual([
      { repo_id: 20, required: true },
      { repo_id: 21, required: true },
    ])
  })

  it('locks every control while a save is in flight', () => {
    const wrapper = mount([{ repo_id: 20, required: true }], { disabled: true })
    expect(wrapper.find('select').attributes('disabled')).toBeDefined()
    expect(wrapper.findAll('button').every((b) => b.attributes('disabled') !== undefined)).toBe(
      true,
    )
  })
})
