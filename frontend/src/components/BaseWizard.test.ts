// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { mount, type VueWrapper } from '@vue/test-utils'
import { defineComponent, h, reactive, ref, type Ref } from 'vue'
import BaseWizard, { type WizardStep } from './BaseWizard.vue'

type StepId = 'one' | 'two' | 'three'

const STEPS: WizardStep<StepId>[] = [
  { id: 'one', label: 'One', sub: 'The first' },
  { id: 'two', label: 'Two' },
  { id: 'three', label: 'Three', sub: 'Create it' },
]

interface Harness {
  wrapper: VueWrapper
  /** What each step still needs; edit it to answer or un-answer a step. */
  needs: Record<StepId, string[]>
  steps: Ref<WizardStep<StepId>[]>
  submitting: Ref<boolean>
}

/**
 * Mounts the wizard with reactive steps and blockers, rendering the current
 * step's id and a button per step that calls the slot's `goTo`.
 */
function harness(needs: Partial<Record<StepId, string[]>> = {}): Harness {
  const state = reactive<Record<StepId, string[]>>({ one: [], two: [], three: [], ...needs })
  const steps = ref<WizardStep<StepId>[]>([...STEPS])
  const submitting = ref(false)
  const Host = defineComponent({
    setup() {
      return () =>
        h(
          BaseWizard,
          {
            steps: steps.value,
            blockers: (id: StepId) => state[id],
            label: 'Thing steps',
            submitLabel: 'Create thing',
            submitting: submitting.value,
          },
          {
            default: ({
              step,
              goTo,
              blocked,
            }: {
              step: StepId
              goTo: (id: StepId) => void
              blocked: boolean
            }) => [
              h('p', { class: 'current-pane' }, step),
              h('p', { class: 'pane-blocked' }, String(blocked)),
              ...STEPS.map((s) =>
                h(
                  'button',
                  { class: `go-${s.id}`, type: 'button', onClick: () => goTo(s.id) },
                  s.id,
                ),
              ),
              h(
                'button',
                { class: 'go-missing', type: 'button', onClick: () => goTo('nope' as StepId) },
                'missing',
              ),
            ],
          },
        )
    },
  })
  return { wrapper: mount(Host), needs: state, steps, submitting }
}

function wizard(wrapper: VueWrapper): VueWrapper {
  return wrapper.findComponent(BaseWizard) as unknown as VueWrapper
}

function button(wrapper: VueWrapper, label: string) {
  const match = wrapper.findAll('.wizard-actions button').find((b) => b.text().trim() === label)
  if (!match) throw new Error(`no button "${label}"`)
  return match
}

function pane(wrapper: VueWrapper): string {
  return wrapper.get('.current-pane').text()
}

function status(wrapper: VueWrapper): string {
  return wrapper.get('.wizard-status').text()
}

/** Each rail step's number badge state, from its `wizard-num--*` modifier. */
function states(wrapper: VueWrapper): string[] {
  return wrapper.findAll('.wizard-num').map((n) => {
    const modifier = n.classes().find((c) => c.startsWith('wizard-num--'))
    return modifier!.replace('wizard-num--', '')
  })
}

async function railClick(wrapper: VueWrapper, index: number): Promise<void> {
  await wrapper.findAll('.wizard-step')[index]!.trigger('click')
}

describe('BaseWizard', () => {
  it('lists every step in a labelled rail, starting on the first', () => {
    const { wrapper } = harness()
    expect(wrapper.get('nav').attributes('aria-label')).toBe('Thing steps')
    const items = wrapper.findAll('.wizard-step')
    expect(items.map((i) => i.get('.wizard-step-text > span').text())).toEqual([
      'One',
      'Two',
      'Three',
    ])
    expect(items.map((i) => i.find('.wizard-step-sub').exists())).toEqual([true, false, true])
    expect(items[0]!.get('.wizard-step-sub').text()).toBe('The first')
    expect(items.map((i) => i.attributes('aria-current'))).toEqual(['true', 'false', 'false'])
    expect(items[0]!.classes()).toContain('wizard-step--current')
    expect(wrapper.get('.wizard-caption').text()).toBe('Step 1 of 3 · One')
    expect(pane(wrapper)).toBe('one')
  })

  it('moves on with Continue and back with Back, which is off on the first step', async () => {
    const { wrapper } = harness()
    expect(button(wrapper, 'Back').attributes('disabled')).toBeDefined()
    expect(status(wrapper)).toBe('Step 1 of 3.')

    await button(wrapper, 'Continue').trigger('click')
    expect(pane(wrapper)).toBe('two')
    expect(wrapper.get('.wizard-caption').text()).toBe('Step 2 of 3 · Two')
    expect(button(wrapper, 'Back').attributes('disabled')).toBeUndefined()

    await button(wrapper, 'Back').trigger('click')
    expect(pane(wrapper)).toBe('one')
  })

  it('swaps Continue for the submit action on the last step only', async () => {
    const { wrapper } = harness()
    expect(wrapper.findAll('.wizard-actions button').map((b) => b.text().trim())).toEqual([
      'Cancel',
      'Back',
      'Continue',
    ])
    await railClick(wrapper, 2)
    expect(wrapper.findAll('.wizard-actions button').map((b) => b.text().trim())).toEqual([
      'Cancel',
      'Back',
      'Create thing',
    ])
  })

  it('holds Continue while the current step still needs something, and says what', async () => {
    const { wrapper, needs } = harness({ one: ['a name', 'an address'] })
    expect(button(wrapper, 'Continue').attributes('disabled')).toBeDefined()
    expect(status(wrapper)).toBe('Needs a name and an address to continue.')
    expect(wrapper.get('.wizard-status').classes()).toContain('wizard-status--blocked')

    needs.one = []
    await wrapper.vm.$nextTick()
    expect(button(wrapper, 'Continue').attributes('disabled')).toBeUndefined()
    expect(status(wrapper)).toBe('Step 1 of 3.')
    expect(wrapper.get('.wizard-status').classes()).not.toContain('wizard-status--blocked')
  })

  it('lets the rail jump to any step, even past a blocked one', async () => {
    const { wrapper } = harness({ one: ['a name'] })
    await railClick(wrapper, 1)
    expect(pane(wrapper)).toBe('two')
    // Step two has no blockers of its own, so Continue is open there.
    expect(button(wrapper, 'Continue').attributes('disabled')).toBeUndefined()
  })

  it('submits from the last step once every step is answered', async () => {
    const { wrapper } = harness()
    await railClick(wrapper, 2)
    const submit = button(wrapper, 'Create thing')
    expect(submit.attributes('disabled')).toBeUndefined()
    await submit.trigger('click')
    expect(wizard(wrapper).emitted('submit')).toHaveLength(1)
  })

  it('holds the submit action while any step needs something, naming each and its step', async () => {
    const { wrapper, needs } = harness({ one: ['a name'], two: ['a port'] })
    await railClick(wrapper, 2)
    const submit = button(wrapper, 'Create thing')
    expect(submit.attributes('disabled')).toBeDefined()
    expect(status(wrapper)).toBe('Still needs a name (One), a port (Two).')

    // A click that slips through a disabled button still does not submit.
    ;(submit.element as HTMLButtonElement).disabled = false
    await submit.trigger('click')
    expect(wizard(wrapper).emitted('submit')).toBeUndefined()

    needs.one = []
    needs.two = []
    await wrapper.vm.$nextTick()
    expect(button(wrapper, 'Create thing').attributes('disabled')).toBeUndefined()
    expect(status(wrapper)).toBe('Step 3 of 3.')
  })

  it('reports the last step blocked by its own needs too', async () => {
    const { wrapper } = harness({ three: ['a confirmation'] })
    await railClick(wrapper, 2)
    expect(status(wrapper)).toBe('Still needs a confirmation (Three).')
  })

  it('shows the submitting label and does not submit twice while submitting', async () => {
    const { wrapper, submitting } = harness()
    await railClick(wrapper, 2)
    submitting.value = true
    await wrapper.vm.$nextTick()

    const busy = button(wrapper, 'Creating...')
    expect(busy.attributes('disabled')).toBeDefined()
    ;(busy.element as HTMLButtonElement).disabled = false
    await busy.trigger('click')
    expect(wizard(wrapper).emitted('submit')).toBeUndefined()
  })

  it('takes a custom submitting label', async () => {
    const wrapper = mount(BaseWizard, {
      props: {
        steps: [{ id: 'only', label: 'Only' }],
        blockers: () => [],
        label: 'Steps',
        submitLabel: 'Save',
        submittingLabel: 'Saving...',
        submitting: true,
      },
    })
    expect(wrapper.find('.wizard-actions .btn-primary').text()).toBe('Saving...')
    // A single step is both the first and the last.
    expect(
      wrapper
        .findAll('.wizard-actions button')
        .find((b) => b.text().includes('Back'))!
        .attributes('disabled'),
    ).toBeDefined()
  })

  it('emits cancel from Cancel on any step', async () => {
    const { wrapper } = harness({ one: ['a name'] })
    await button(wrapper, 'Cancel').trigger('click')
    await railClick(wrapper, 2)
    await button(wrapper, 'Cancel').trigger('click')
    expect(wizard(wrapper).emitted('cancel')).toHaveLength(2)
    expect(wizard(wrapper).emitted('submit')).toBeUndefined()
  })

  it('marks steps as current, done, todo once visited, or blank when not yet seen', async () => {
    const { wrapper, needs } = harness({ two: ['a port'], three: ['a review'] })
    // An answered step is done before it is visited; an unanswered one stays blank.
    expect(states(wrapper)).toEqual(['current', 'blank', 'blank'])

    await railClick(wrapper, 1)
    expect(states(wrapper)).toEqual(['done', 'current', 'blank'])

    await railClick(wrapper, 2)
    // Two was seen and left unanswered.
    expect(states(wrapper)).toEqual(['done', 'todo', 'current'])

    await railClick(wrapper, 0)
    expect(states(wrapper)).toEqual(['current', 'todo', 'todo'])

    needs.two = []
    await wrapper.vm.$nextTick()
    expect(states(wrapper)).toEqual(['current', 'done', 'todo'])
  })

  it('shows a tick instead of the number on a done step', async () => {
    const { wrapper } = harness({ three: ['a review'] })
    await railClick(wrapper, 1)
    const nums = wrapper.findAll('.wizard-num')
    expect(nums[0]!.find('svg').exists()).toBe(true)
    expect(nums[0]!.text()).toBe('')
    expect(nums[1]!.text()).toBe('2')
    expect(nums[2]!.text()).toBe('3')
  })

  it('jumps to a step by id from the pane, and ignores an unknown id', async () => {
    const { wrapper } = harness()
    await wrapper.get('.go-three').trigger('click')
    expect(pane(wrapper)).toBe('three')
    expect(wrapper.findAll('.wizard-step')[2]!.attributes('aria-current')).toBe('true')

    await wrapper.get('.go-missing').trigger('click')
    expect(pane(wrapper)).toBe('three')

    await wrapper.get('.go-one').trigger('click')
    expect(pane(wrapper)).toBe('one')
  })

  it('stays inside the steps when they shrink under the current one', async () => {
    const { wrapper, steps } = harness()
    await railClick(wrapper, 2)
    expect(pane(wrapper)).toBe('three')

    steps.value = STEPS.slice(0, 2)
    await wrapper.vm.$nextTick()
    await wrapper.vm.$nextTick()
    expect(pane(wrapper)).toBe('two')
    expect(wrapper.findAll('.wizard-step')).toHaveLength(2)
    expect(wrapper.get('.wizard-caption').text()).toBe('Step 2 of 2 · Two')
    // Two is now the last step, so it offers the submit action.
    expect(button(wrapper, 'Create thing').exists()).toBe(true)
  })

  it('keeps the current step when the steps shrink after it', async () => {
    const { wrapper, steps } = harness()
    steps.value = STEPS.slice(0, 2)
    await wrapper.vm.$nextTick()
    expect(pane(wrapper)).toBe('one')
    expect(wrapper.get('.wizard-caption').text()).toBe('Step 1 of 2 · One')
  })

  it('tells the pane whether any step is still blocked', async () => {
    const { wrapper, needs } = harness({ two: ['a name'] })
    expect(wrapper.find('.pane-blocked').text()).toBe('true')

    needs.two = []
    await wrapper.vm.$nextTick()
    expect(wrapper.find('.pane-blocked').text()).toBe('false')
  })
})
