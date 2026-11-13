// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { AxiosError } from 'axios'
import type { AxiosResponse } from 'axios'
import { mount, flushPromises, type VueWrapper } from '@vue/test-utils'
import type { ComponentPublicInstance } from 'vue'
import type {
  DeployStatusResponse,
  DeployStep,
  DeployStepResponse,
  DeployStepStatus,
} from '../types/generated'
import AgentDeployDialog from './AgentDeployDialog.vue'
import BaseModal from './BaseModal.vue'

const postMock = vi.fn()
const getMock = vi.fn()
const deleteMock = vi.fn()

vi.mock('../api/client', () => ({
  apiClient: {
    post: (...args: unknown[]): unknown => postMock(...args),
    get: (...args: unknown[]): unknown => getMock(...args),
    delete: (...args: unknown[]): unknown => deleteMock(...args),
  },
}))

vi.mock('../utils/error', () => ({
  extractError: (_e: unknown): string => 'API error',
}))

const STEP_ORDER: DeployStep[] = ['connect', 'upload', 'install', 'restart', 'reconnect']

/** The five steps, the first ones as given and the rest pending. */
function stepList(statuses: DeployStepStatus[]): DeployStepResponse[] {
  return STEP_ORDER.map((step, i) => ({
    step,
    status: statuses[i] ?? 'pending',
    duration_ms: statuses[i] === 'done' ? 400 : null,
  }))
}

function deployStatus(overrides: Partial<DeployStatusResponse> = {}): DeployStatusResponse {
  return {
    state: 'running',
    steps: stepList(['done', 'running']),
    skipped: false,
    token: null,
    available_version: null,
    error: null,
    ...overrides,
  }
}

function notFound(): AxiosError {
  return new AxiosError(
    'Request failed with status code 404',
    AxiosError.ERR_BAD_REQUEST,
    undefined,
    undefined,
    {
      status: 404,
      statusText: 'Not Found',
      data: {},
      headers: {},
      config: {},
    } as AxiosResponse,
  )
}

let wrapper: VueWrapper<ComponentPublicInstance> | null = null

function mountDialog(props: {
  hostname: string
  agentVersion: string | null
  availableVersion?: string | null
  lastSshUser?: string | null
}): VueWrapper<ComponentPublicInstance> {
  wrapper = mount(AgentDeployDialog, {
    props,
    attachTo: document.body,
  })
  return wrapper
}

describe('AgentDeployDialog', () => {
  beforeEach(() => {
    postMock.mockReset()
    getMock.mockReset()
    deleteMock.mockReset()
    postMock.mockResolvedValue({ data: { content: null } })
    // No deploy is running or recent, unless a test says so.
    getMock.mockRejectedValue(notFound())
    deleteMock.mockResolvedValue({})
  })

  afterEach(() => {
    wrapper?.unmount()
    wrapper = null
  })

  it('defaults SSH user to root when no last-used username is known', async () => {
    mountDialog({ hostname: 'web-server-01', agentVersion: '1.0.0' })
    await flushPromises()
    const sshUserInput = document.querySelector<HTMLInputElement>('input[placeholder="root"]')
    expect(sshUserInput?.value).toBe('root')
  })

  // `systemctl show` gives every user on the host a unit's environment, so the
  // default unit loads the token from a root-only file instead of holding it.
  it('loads the agent token from the token file in the default unit', async () => {
    mountDialog({ hostname: 'web-server-01', agentVersion: '1.0.0' })
    await flushPromises()
    const unit = document.querySelector<HTMLTextAreaElement>('textarea')?.value
    expect(unit).toContain('EnvironmentFile=/etc/assimilate-agent.env')
    expect(unit).not.toContain('BORG_AGENT_TOKEN')
  })

  it('prefills SSH user from the lastSshUser prop', async () => {
    mountDialog({
      hostname: 'web-server-01',
      agentVersion: '1.0.0',
      lastSshUser: 'deploy-user',
    })
    await flushPromises()
    const sshUserInput = document.querySelector<HTMLInputElement>('input[placeholder="root"]')
    expect(sshUserInput?.value).toBe('deploy-user')
  })

  it('automatically loads the existing service unit from the remote host on mount', async () => {
    postMock.mockResolvedValue({
      data: { content: '[Service]\nEnvironment=BORG_AGENT_TOKEN=[REDACTED]\n' },
    })
    mountDialog({ hostname: 'web-server-01', agentVersion: '1.0.0' })
    await flushPromises()
    expect(postMock).toHaveBeenCalledWith(
      '/agents/web-server-01/service-unit',
      expect.objectContaining({ ssh_host: 'web-server-01' }),
    )
    const textarea = document.querySelector<HTMLTextAreaElement>('textarea')
    expect(textarea?.value).toContain('Environment=BORG_AGENT_TOKEN=[REDACTED]')
  })

  it('does not surface an error banner when the automatic load finds no remote unit', async () => {
    postMock.mockResolvedValue({ data: { content: null } })
    mountDialog({ hostname: 'web-server-01', agentVersion: '1.0.0' })
    await flushPromises()
    expect(document.querySelector('.field-hint-error')).toBeNull()
  })

  it('does not surface an error banner when the automatic load fails to connect', async () => {
    postMock.mockRejectedValue(new Error('connection refused'))
    mountDialog({ hostname: 'web-server-01', agentVersion: '1.0.0' })
    await flushPromises()
    expect(document.querySelector('.field-hint-error')).toBeNull()
  })

  it('shows an error banner when a manual "Load from remote" click finds no unit', async () => {
    postMock.mockResolvedValue({ data: { content: null } })
    mountDialog({ hostname: 'web-server-01', agentVersion: '1.0.0' })
    await flushPromises()
    const loadBtn = Array.from(document.querySelectorAll<HTMLButtonElement>('button')).find((b) =>
      b.textContent?.includes('Load from remote'),
    )
    loadBtn?.click()
    await flushPromises()
    expect(document.querySelector('.field-hint-error')?.textContent).toContain(
      'No existing service unit found',
    )
  })

  it('does not clobber in-progress edits with the result of the automatic load', async () => {
    let resolvePost: (value: { data: { content: string | null } }) => void = () => {}
    postMock.mockReturnValue(
      new Promise((resolve) => {
        resolvePost = resolve
      }),
    )
    mountDialog({ hostname: 'web-server-01', agentVersion: '1.0.0' })
    await flushPromises()

    const textarea = document.querySelector<HTMLTextAreaElement>('textarea')
    expect(textarea).not.toBeNull()
    textarea!.value = 'user in-progress edit'
    textarea!.dispatchEvent(new Event('input'))
    await flushPromises()

    resolvePost({ data: { content: '[Service]\nEnvironment=BORG_AGENT_TOKEN=[REDACTED]\n' } })
    await flushPromises()

    expect(textarea?.value).toBe('user in-progress edit')
  })

  describe('version summary', () => {
    it('shows the installed and available versions and names the target on the submit button', () => {
      mountDialog({ hostname: 'web-server-01', agentVersion: '1.0.0', availableVersion: '1.2.0' })
      expect(document.querySelector('.upgrade-hero')?.textContent).toContain('1.0.0')
      expect(document.querySelector('.upgrade-hero')?.textContent).toContain('1.2.0')
      const submitBtn = Array.from(document.querySelectorAll<HTMLButtonElement>('button')).find(
        (b) => b.textContent?.includes('Upgrade to'),
      )
      expect(submitBtn?.textContent?.trim()).toBe('Upgrade to 1.2.0')
    })

    it('falls back to a generic label when the two version strings are identical', () => {
      // A dev build can be newer without its semantic version changing (the
      // server compares by git commit count instead).
      mountDialog({ hostname: 'web-server-01', agentVersion: '1.0.0', availableVersion: '1.0.0' })
      expect(document.querySelector('.upgrade-hero')?.textContent).toContain(
        'A newer build is available.',
      )
      const submitBtn = Array.from(document.querySelectorAll<HTMLButtonElement>('button')).find(
        (b) => b.textContent?.trim() === 'Upgrade Agent',
      )
      expect(submitBtn).toBeDefined()
    })

    it('shows no version hero for a first-time deploy', () => {
      mountDialog({ hostname: 'web-server-01', agentVersion: null })
      expect(document.querySelector('.upgrade-hero')).toBeNull()
    })
  })

  describe('service unit disclosure', () => {
    function isOpen(): boolean {
      const body = document.querySelector<HTMLElement>('.disclosure-body')
      return body !== null && body.style.display !== 'none'
    }

    function toggle(): void {
      document.querySelector<HTMLButtonElement>('.disclosure-head')?.click()
    }

    it('starts collapsed on an upgrade, where a working unit already exists', async () => {
      mountDialog({ hostname: 'web-server-01', agentVersion: '1.0.0' })
      await flushPromises()
      expect(isOpen()).toBe(false)
    })

    // A first-time deploy has no established unit yet, so loading or editing
    // one is part of setting the host up rather than a detail to scroll past.
    it('starts open on a first-time deploy', async () => {
      mountDialog({ hostname: 'web-server-01', agentVersion: null })
      await flushPromises()
      expect(isOpen()).toBe(true)
    })

    it('opens and closes on click', async () => {
      mountDialog({ hostname: 'web-server-01', agentVersion: '1.0.0' })
      await flushPromises()

      toggle()
      await flushPromises()
      expect(isOpen()).toBe(true)

      toggle()
      await flushPromises()
      expect(isOpen()).toBe(false)
    })

    it('badges the unit Default until its content diverges from the generated default', async () => {
      mountDialog({ hostname: 'web-server-01', agentVersion: '1.0.0' })
      await flushPromises()
      const badge = (): string | undefined =>
        document.querySelector('.disclosure-head .badge')?.textContent?.trim()
      expect(badge()).toBe('Default')

      const textarea = document.querySelector<HTMLTextAreaElement>('textarea')!
      textarea.value = '# a hand-edited unit'
      textarea.dispatchEvent(new Event('input'))
      await flushPromises()

      expect(badge()).toBe('Customized')
    })
  })

  // Escape and the backdrop reach the dialog as BaseModal's close event, which
  // is wired separately from the Cancel button and has to behave the same.
  it.each([
    [
      'Cancel is clicked',
      (w: VueWrapper<ComponentPublicInstance>): void => {
        void w
        Array.from(document.querySelectorAll<HTMLButtonElement>('button'))
          .find((b) => b.textContent?.trim() === 'Cancel')
          ?.click()
      },
    ],
    [
      'the modal is dismissed',
      (w: VueWrapper<ComponentPublicInstance>): void => {
        w.findComponent(BaseModal).vm.$emit('close')
      },
    ],
  ])('emits close when %s', async (_how, dismiss) => {
    const w = mountDialog({ hostname: 'web-server-01', agentVersion: null })
    await flushPromises()
    dismiss(w)
    await w.vm.$nextTick()
    expect(w.emitted('close')).toHaveLength(1)
  })

  describe('deploy form', () => {
    /** The dialog teleports, so its fields are queried off the document. */
    async function setField(label: string, value: string): Promise<void> {
      const wrap = [...document.querySelectorAll('.field')].find((f) =>
        f.querySelector('.field-label')?.textContent?.includes(label),
      )
      const control = wrap?.querySelector<HTMLInputElement>('input, select, textarea')
      if (!control) throw new Error(`no field labelled "${label}"`)
      control.value = value
      control.dispatchEvent(new Event('input'))
      control.dispatchEvent(new Event('change'))
      await flushPromises()
    }

    async function submit(): Promise<void> {
      const button = [...document.querySelectorAll<HTMLButtonElement>('button')].find((b) =>
        /Deploy|Install|Upgrade/.test(b.textContent ?? ''),
      )
      if (!button) throw new Error('no deploy button')
      button.click()
      await flushPromises()
    }

    /** Fills in the form for `hostname` and starts the deploy the dialog was mounted for. */
    async function startDeploy(): Promise<void> {
      await setField('SSH host', '10.0.0.5')
      await setField('Server URL', 'https://assimilate.example.com')
      await submit()
    }

    /** Lets the dialog's next poll of the deploy status happen. */
    async function nextPoll(): Promise<void> {
      await vi.advanceTimersByTimeAsync(1000)
      await flushPromises()
    }

    /** Mounts an upgrade dialog, ready to be submitted. */
    async function mountUpgrade(): Promise<VueWrapper<ComponentPublicInstance>> {
      const w = mountDialog({
        hostname: 'web-server-01',
        agentVersion: '1.0.0',
        availableVersion: '1.2.0',
      })
      await flushPromises()
      return w
    }

    it('sends every field it collected', async () => {
      const post = postMock
      post.mockResolvedValue({ data: deployStatus({ state: 'succeeded', skipped: true }) })

      mountDialog({ hostname: 'web-server-01', agentVersion: null })
      await flushPromises()
      post.mockClear()

      await setField('SSH host', '10.0.0.5')
      await setField('SSH user', 'deployer')
      await setField('SSH port', '2222')
      await setField('SSH password', 'hunter2')
      await setField('Server URL', 'https://assimilate.example.com')
      await setField('Install path', '/opt/assimilate')

      await submit()

      expect(post).toHaveBeenCalledWith(
        '/agents/web-server-01/deploy',
        expect.objectContaining({
          ssh_host: '10.0.0.5',
          ssh_user: 'deployer',
          ssh_port: 2222,
          ssh_password: 'hunter2',
          server_url: 'https://assimilate.example.com',
          install_path: '/opt/assimilate',
        }),
        { params: {} },
      )
    })

    // The optional fields are sent as undefined rather than as empty strings,
    // so the server applies its own defaults instead of writing blanks.
    it('omits the optional fields when they are left empty', async () => {
      const post = postMock
      post.mockResolvedValue({ data: deployStatus({ state: 'succeeded', skipped: true }) })

      mountDialog({ hostname: 'web-server-01', agentVersion: null })
      await flushPromises()
      post.mockClear()

      await setField('SSH host', '10.0.0.5')
      await setField('Server URL', 'https://assimilate.example.com')
      await setField('Install path', '')

      await submit()

      const body = post.mock.calls[0][1] as Record<string, unknown>
      expect(body.ssh_password).toBeUndefined()
      expect(body.install_path).toBeUndefined()
    })

    describe('a deploy that runs on the server', () => {
      beforeEach(() => {
        vi.useFakeTimers()
      })

      afterEach(() => {
        vi.useRealTimers()
      })

      it('answers at once with the steps, and keeps following them until the deploy ends', async () => {
        const w = await mountUpgrade()
        postMock.mockResolvedValueOnce({ data: deployStatus() })
        getMock
          .mockResolvedValueOnce({
            data: deployStatus({ steps: stepList(['done', 'done', 'running']) }),
          })
          .mockResolvedValueOnce({
            data: deployStatus({
              state: 'succeeded',
              steps: stepList(['done', 'done', 'done', 'done', 'done']),
              token: 'tok-abc123',
              available_version: '1.2.0',
            }),
          })

        await startDeploy()

        // The request has answered, and the form has given way to the steps.
        expect(document.querySelectorAll('.deploy-step')).toHaveLength(5)
        expect(document.body.textContent).toContain('Upgrading to')
        expect(document.body.textContent).toContain('You can close this window')
        expect(document.querySelector('.deploy-step .progress-bar')).toBeNull()
        expect(w.emitted('deployed')).toBeUndefined()

        await nextPoll()
        expect(document.querySelectorAll('.deploy-step-icon--done')).toHaveLength(2)
        expect(w.emitted('deployed')).toBeUndefined()

        await nextPoll()
        expect(w.emitted('deployed')).toEqual([['1.2.0']])
        expect(document.querySelector('.token-text')?.textContent).toBe('tok-abc123')
        expect(document.body.textContent).toContain('Upgraded to 1.2.0')
      })

      it('stops asking once the deploy has ended', async () => {
        await mountUpgrade()
        postMock.mockResolvedValueOnce({ data: deployStatus() })
        getMock.mockResolvedValue({
          data: deployStatus({
            state: 'failed',
            error: 'boom',
            steps: stepList(['done', 'failed']),
          }),
        })

        await startDeploy()
        await nextPoll()
        const asked = getMock.mock.calls.length
        await nextPoll()
        await nextPoll()

        expect(getMock).toHaveBeenCalledTimes(asked)
      })

      it("shows a failure in the step it failed in, with the server's reason beside the buttons", async () => {
        const w = await mountUpgrade()
        postMock.mockResolvedValueOnce({ data: deployStatus() })
        getMock.mockResolvedValue({
          data: deployStatus({
            state: 'failed',
            steps: stepList(['done', 'done', 'failed']),
            error: 'failed to install the agent binary (exit 1): Text file busy',
          }),
        })

        await startDeploy()
        await nextPoll()

        expect(document.querySelectorAll('.deploy-step-icon--failed')).toHaveLength(1)
        expect(document.body.textContent).toContain('Nothing was restarted')
        const error = document.querySelector('.modal-footer .form-error')
        expect(error?.textContent).toContain('Text file busy')
        expect(document.querySelector('.modal-body .form-error')).toBeNull()
        expect(w.emitted('deployed')).toBeUndefined()
      })

      it('says the new binary is already in place when the failure came after the install', async () => {
        await mountUpgrade()
        postMock.mockResolvedValueOnce({ data: deployStatus() })
        getMock.mockResolvedValue({
          data: deployStatus({
            state: 'failed',
            steps: stepList(['done', 'done', 'done', 'done', 'failed']),
            error: 'the agent did not connect back',
          }),
        })

        await startDeploy()
        await nextPoll()

        expect(document.body.textContent).toContain('already on the host')
        expect(document.body.textContent).not.toContain('Nothing was restarted')
      })

      it("goes back to the form on Try again, dropping the failed deploy's token", async () => {
        await mountUpgrade()
        postMock.mockResolvedValueOnce({ data: deployStatus() })
        getMock.mockResolvedValue({
          data: deployStatus({
            state: 'failed',
            steps: stepList(['done', 'failed']),
            error: 'boom',
          }),
        })
        await startDeploy()
        await nextPoll()

        const again = [...document.querySelectorAll<HTMLButtonElement>('button')].find(
          (b) => b.textContent?.trim() === 'Try again',
        )
        again?.click()
        await flushPromises()

        expect(deleteMock).toHaveBeenCalledWith('/agents/web-server-01/deploy', { params: {} })
        expect(document.querySelector('input[placeholder="root"]')).not.toBeNull()
        expect(document.querySelector('.deploy-steps')).toBeNull()
      })

      it('leaves a running deploy alone when the dialog is closed', async () => {
        const w = await mountUpgrade()
        postMock.mockResolvedValueOnce({ data: deployStatus() })
        await startDeploy()

        const close = [...document.querySelectorAll<HTMLButtonElement>('button')].find(
          (b) => b.textContent?.trim() === 'Close' && b.classList.contains('btn'),
        )
        close?.click()
        await flushPromises()

        expect(w.emitted('close')).toHaveLength(1)
        expect(deleteMock).not.toHaveBeenCalled()
      })

      it("drops a finished deploy's token when Done is pressed", async () => {
        const w = await mountUpgrade()
        postMock.mockResolvedValueOnce({ data: deployStatus() })
        getMock.mockResolvedValue({
          data: deployStatus({
            state: 'succeeded',
            steps: stepList(['done', 'done', 'done', 'done', 'done']),
            token: 'tok-abc123',
            available_version: '1.2.0',
          }),
        })
        await startDeploy()
        await nextPoll()

        const done = [...document.querySelectorAll<HTMLButtonElement>('button')].find(
          (b) => b.textContent?.trim() === 'Done',
        )
        done?.click()
        await flushPromises()

        expect(deleteMock).toHaveBeenCalledTimes(1)
        expect(w.emitted('close')).toHaveLength(1)
      })

      it('reports an agent that is already current without waiting for anything', async () => {
        const w = await mountUpgrade()
        postMock.mockResolvedValueOnce({
          data: deployStatus({
            state: 'succeeded',
            skipped: true,
            steps: stepList([]),
            available_version: '1.2.0',
          }),
        })
        getMock.mockClear()

        await startDeploy()

        expect(document.body.textContent).toContain('Already on 1.2.0')
        expect(w.emitted('deployed')).toBeUndefined()
        await nextPoll()
        expect(getMock).not.toHaveBeenCalled()
      })

      it('keeps polling through a failed poll, and gives up after several in a row', async () => {
        await mountUpgrade()
        postMock.mockResolvedValueOnce({ data: deployStatus() })
        getMock.mockRejectedValue(new Error('offline'))
        await startDeploy()

        await nextPoll()
        expect(document.querySelector('.deploy-steps')).not.toBeNull()
        for (let i = 0; i < 5; i++) await nextPoll()

        // extractError is stubbed to a fixed string in this file's mocks.
        expect(document.querySelector('.modal-footer .form-error')?.textContent).toContain(
          'API error',
        )
        const asked = getMock.mock.calls.length
        await nextPoll()
        expect(getMock).toHaveBeenCalledTimes(asked)
      })
    })

    it('picks up a deploy that is already running when it opens', async () => {
      getMock.mockReset()
      getMock.mockResolvedValue({ data: deployStatus() })

      mountDialog({ hostname: 'web-server-01', agentVersion: '1.0.0', availableVersion: '1.2.0' })
      await flushPromises()

      expect(document.querySelectorAll('.deploy-step')).toHaveLength(5)
      expect(document.querySelector('input[placeholder="root"]')).toBeNull()
    })

    it('does not announce a deploy it only found already finished', async () => {
      getMock.mockReset()
      getMock.mockResolvedValue({
        data: deployStatus({
          state: 'succeeded',
          steps: stepList(['done', 'done', 'done', 'done', 'done']),
          token: 'tok-abc123',
          available_version: '1.2.0',
        }),
      })

      const w = mountDialog({
        hostname: 'web-server-01',
        agentVersion: '1.0.0',
        availableVersion: '1.2.0',
      })
      await flushPromises()

      expect(document.querySelector('.token-text')?.textContent).toBe('tok-abc123')
      expect(w.emitted('deployed')).toBeUndefined()
    })

    it('tells a user other than the one who started the deploy where the token went', async () => {
      getMock.mockReset()
      getMock.mockResolvedValue({
        data: deployStatus({
          state: 'succeeded',
          steps: stepList(['done', 'done', 'done', 'done', 'done']),
          token: null,
          available_version: '1.2.0',
        }),
      })

      mountDialog({ hostname: 'web-server-01', agentVersion: '1.0.0', availableVersion: '1.2.0' })
      await flushPromises()

      expect(document.querySelector('.token-text')).toBeNull()
      expect(document.body.textContent).toContain('only shown to the user who started the deploy')
    })

    it('reports a request that failed rather than claiming success', async () => {
      const post = postMock
      post.mockResolvedValue({ data: {} })

      const w = mountDialog({ hostname: 'web-server-01', agentVersion: null })
      await flushPromises()
      post.mockRejectedValueOnce(new Error('ssh refused'))

      await setField('SSH host', '10.0.0.5')
      await setField('Server URL', 'https://assimilate.example.com')
      await submit()

      expect(w.emitted('deployed')).toBeUndefined()
      // extractError is stubbed to a fixed string in this file's mocks.
      expect(document.body.textContent).toContain('API error')
      const error = document.querySelector('.modal-footer .form-error')
      expect(error?.textContent).toContain('API error')
      expect(document.querySelector('.modal-body .form-error')).toBeNull()
    })
  })
})
