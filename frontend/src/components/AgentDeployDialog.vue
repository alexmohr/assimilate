<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, reactive, computed, onMounted } from 'vue'
import { ArrowRight, Upload, KeyRound, CheckCircle } from '@lucide/vue'
import { previewAgentServiceUnit, deployAgent } from '../api/agents'
import type { DeployAgentResult, ServiceUnitPreviewResponse } from '../api/agents'
import { useEscapeKey } from '../composables/useEscapeKey'
import { extractError } from '../utils/error'
import BaseModal from './BaseModal.vue'
import BaseDisclosure from './BaseDisclosure.vue'

const DEFAULT_INSTALL_PATH = '/usr/local/bin/assimilate-agent'

const props = defineProps<{
  hostname: string
  /** Disambiguates `hostname` when it is shared by more than one agent. */
  domain?: string | null
  agentVersion: string | null
  /** The version installable from the server, when known - drives the version-transition summary. */
  availableVersion?: string | null
  lastSshUser?: string | null
  /**
   * Reinstall even though the agent already reports the latest version -
   * for a host that was reimaged or otherwise lost its installation.
   */
  forceRedeploy?: boolean
}>()

const emit = defineEmits<{
  close: []
  deployed: [version: string | null]
}>()

const visible = ref(true)

useEscapeKey(visible, () => {
  emit('close')
})

const isRedeploy = computed(() => props.forceRedeploy === true)
const isUpgrade = computed(() => !isRedeploy.value && props.agentVersion !== null)
// A build can be newer without the semantic version string changing (a dev
// server compared by commit count) - the arrow-to-version layout only makes
// sense when the two strings actually differ.
const showAvailableVersion = computed(
  () => !!props.availableVersion && props.availableVersion !== props.agentVersion,
)

const deployLoading = ref(false)
const deployError = ref<string | null>(null)
const fetchServiceLoading = ref(false)
const fetchServiceError = ref<string | null>(null)
const serviceContentTouched = ref(false)
const deployResult = ref<DeployAgentResult | null>(null)

const deployForm = reactive({
  ssh_host: '',
  ssh_user: 'root',
  ssh_port: 22,
  ssh_password: '',
  server_url: '',
  install_path: DEFAULT_INSTALL_PATH,
  systemd_service_content: '',
})

function defaultSystemdUnit(execPath: string): string {
  return `[Unit]
Description=Assimilate Backup Agent
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=${execPath}
Environment=BORG_SERVER_URL=<will be set automatically>
Environment=BORG_AGENT_TOKEN=<will be set automatically>
Restart=on-failure
RestartSec=10

[Install]
WantedBy=multi-user.target
`
}

// Drives the Default / Customized badge on the collapsed disclosure, so a
// user can tell whether there's anything non-standard inside without opening it.
const hasCustomUnit = computed(() => {
  const expected = defaultSystemdUnit(deployForm.install_path.trim() || DEFAULT_INSTALL_PATH)
  return deployForm.systemd_service_content.trim() !== expected.trim()
})

onMounted(() => {
  deployForm.ssh_host = props.hostname
  deployForm.ssh_user = props.lastSshUser || 'root'
  deployForm.server_url = window.location.origin
  deployForm.systemd_service_content = defaultSystemdUnit(DEFAULT_INSTALL_PATH)
  void loadExistingServiceUnit({ silent: true })
})

async function loadExistingServiceUnit(options: { silent?: boolean } = {}): Promise<void> {
  if (!deployForm.ssh_host) return
  fetchServiceLoading.value = true
  if (!options.silent) fetchServiceError.value = null
  try {
    const res: ServiceUnitPreviewResponse = await previewAgentServiceUnit(props.hostname, {
      ssh_host: deployForm.ssh_host.trim(),
      ssh_user: deployForm.ssh_user.trim(),
      ssh_port: deployForm.ssh_port,
      ssh_password: deployForm.ssh_password || undefined,
    })
    // A silent (auto-triggered) load must not clobber content the user has already
    // started editing while the request was in flight.
    if (options.silent && serviceContentTouched.value) return
    if (res.content) {
      deployForm.systemd_service_content = res.content
    } else if (!options.silent) {
      fetchServiceError.value = 'No existing service unit found on remote host.'
    }
  } catch (e: unknown) {
    if (!options.silent) fetchServiceError.value = extractError(e)
  } finally {
    fetchServiceLoading.value = false
  }
}

function dialogTitle(): string {
  if (isRedeploy.value) return 'Redeploy'
  return props.agentVersion ? 'Upgrade' : 'Deploy'
}

function submitLabel(): string {
  if (deployLoading.value) return isRedeploy.value ? 'Redeploying...' : 'Deploying...'
  if (isRedeploy.value) return 'Redeploy Agent'
  if (isUpgrade.value) {
    return showAvailableVersion.value ? `Upgrade to ${props.availableVersion}` : 'Upgrade Agent'
  }
  return 'Deploy Agent'
}

async function submitDeploy(): Promise<void> {
  deployLoading.value = true
  deployError.value = null
  deployResult.value = null
  try {
    const res = await deployAgent(
      props.hostname,
      {
        ssh_host: deployForm.ssh_host.trim(),
        ssh_user: deployForm.ssh_user.trim(),
        ssh_port: deployForm.ssh_port,
        ssh_password: deployForm.ssh_password || undefined,
        server_url: deployForm.server_url.trim(),
        install_path: deployForm.install_path.trim() || undefined,
        systemd_service_content: deployForm.systemd_service_content.trim() || undefined,
        force: isRedeploy.value || undefined,
      },
      props.domain,
    )
    deployResult.value = res
    if (res.success) {
      emit('deployed', res.available_version)
    }
  } catch (e: unknown) {
    deployError.value = extractError(e)
  } finally {
    deployLoading.value = false
  }
}
</script>

<template>
  <BaseModal
    :open="true"
    size="lg"
    @close="emit('close')"
  >
    <template #header="{ titleId }">
      <h2
        :id="titleId"
        class="modal-title"
      >
        {{ dialogTitle() }} Agent &mdash; {{ hostname }}
      </h2>
    </template>
    <template v-if="!deployResult?.success">
      <template v-if="isRedeploy">
        <p class="deploy-info">
          Reinstall the agent binary and systemd service on this host over SSH — useful after the
          machine was reimaged or the installation was otherwise lost. The agent already reports
          version <span class="mono">{{ agentVersion }}</span
          >; this does not change which version is expected to run.
        </p>
        <ul class="icon-list">
          <li>
            <Upload :size="14" />
            <span
              >The binary is re-uploaded over SSH and the <code>assimilate-agent</code> service is
              reinstalled and restarted.</span
            >
          </li>
          <li>
            <KeyRound :size="14" />
            <span>A new agent token is generated and written into the service unit.</span>
          </li>
          <li>
            <CheckCircle :size="14" />
            <span>Schedules, repositories and existing archives are untouched.</span>
          </li>
        </ul>
      </template>
      <template v-else-if="isUpgrade">
        <div class="upgrade-hero">
          <div class="hero-version">
            <span class="group-label">Installed</span>
            <span class="value mono">{{ agentVersion }}</span>
          </div>
          <template v-if="showAvailableVersion">
            <span class="hero-arrow">
              <ArrowRight :size="16" />
            </span>
            <div class="hero-version">
              <span class="group-label">Available</span>
              <span class="value value--next mono">{{ availableVersion }}</span>
            </div>
          </template>
          <span
            v-else-if="availableVersion"
            class="hero-note"
            >A newer build is available.</span
          >
        </div>
        <ul class="icon-list">
          <li>
            <Upload :size="14" />
            <span
              >The new binary is uploaded over SSH and the <code>assimilate-agent</code> service is
              restarted.</span
            >
          </li>
          <li>
            <KeyRound :size="14" />
            <span>A new agent token is generated and written into the service unit.</span>
          </li>
          <li>
            <CheckCircle :size="14" />
            <span>Schedules, repositories and existing archives are untouched.</span>
          </li>
        </ul>
      </template>
      <template v-else>
        <p class="deploy-info">
          Upload and install the agent binary on the target machine via SSH. Sudo is used
          automatically if available; if you provide an SSH password it is also used for sudo.
        </p>
        <p class="deploy-note">
          This will also install and enable the <code>assimilate-agent</code> systemd service on the
          target machine. You can customize the service unit below.
        </p>
      </template>

      <div class="field">
        <label class="field-label">SSH host <span class="required">*</span></label>
        <input
          v-model="deployForm.ssh_host"
          class="input mono"
          placeholder="e.g. 192.168.1.10"
        />
      </div>
      <div class="field-row">
        <div class="field">
          <label class="field-label">SSH user</label>
          <input
            v-model="deployForm.ssh_user"
            class="input mono"
            placeholder="root"
          />
        </div>
        <div class="field field-narrow">
          <label class="field-label">SSH port</label>
          <input
            v-model.number="deployForm.ssh_port"
            class="input"
            type="number"
            min="1"
            max="65535"
          />
        </div>
      </div>
      <div class="field">
        <label class="field-label">SSH password</label>
        <input
          v-model="deployForm.ssh_password"
          class="input mono"
          type="password"
          placeholder="Leave empty to use SSH key"
        />
        <span class="field-hint"
          >Optional — authenticate with password instead of the server's SSH key</span
        >
      </div>
      <div class="field">
        <label class="field-label">Server URL <span class="required">*</span></label>
        <input
          v-model="deployForm.server_url"
          class="input mono"
          placeholder="http://your-server:8080"
        />
        <span class="field-hint">The URL the agent will connect to</span>
        <span class="field-hint">
          Hosts with an enabled SSH tunnel automatically use that tunnel instead.
        </span>
      </div>
      <div class="field">
        <label class="field-label">Install path</label>
        <input
          v-model="deployForm.install_path"
          class="input mono"
          placeholder="/usr/local/bin/assimilate-agent"
        />
      </div>

      <div class="field">
        <BaseDisclosure
          title="Systemd service unit"
          :badge="hasCustomUnit ? 'Customized' : 'Default'"
          :default-open="!agentVersion"
        >
          <div class="disclosure-actions">
            <button
              class="btn btn-sm btn-ghost"
              type="button"
              :disabled="fetchServiceLoading || !deployForm.ssh_host"
              @click="loadExistingServiceUnit()"
            >
              {{ fetchServiceLoading ? 'Loading...' : 'Load from remote' }}
            </button>
          </div>
          <textarea
            v-model="deployForm.systemd_service_content"
            class="input mono service-textarea"
            rows="12"
            spellcheck="false"
            @input="serviceContentTouched = true"
          />
          <span class="field-hint">
            The <code>BORG_SERVER_URL</code> and <code>BORG_AGENT_TOKEN</code> environment variables
            will be injected automatically if not present in custom content. When loaded from a
            remote host, an existing token is shown as <code>[REDACTED]</code> and replaced with a
            newly generated one on deploy.
          </span>
          <span
            v-if="fetchServiceError"
            class="field-hint field-hint-error"
          >
            {{ fetchServiceError }}
          </span>
        </BaseDisclosure>
      </div>
      <div
        v-if="deployError"
        class="form-error"
      >
        {{ deployError }}
      </div>
      <div
        v-if="deployResult && !deployResult.success"
        class="form-error"
      >
        {{ deployResult.error }}
      </div>
    </template>

    <template v-else>
      <div class="token-notice">
        <template v-if="deployResult.skipped">
          <p class="deploy-skipped-msg">
            Already on {{ deployResult.available_version }}. Nothing was changed and the token was
            not rotated.
          </p>
        </template>
        <template v-else>
          <p class="deploy-success-msg">
            {{
              isRedeploy
                ? 'Agent redeployed successfully.'
                : isUpgrade
                  ? `Upgraded to ${deployResult.available_version ?? 'the latest version'}.`
                  : 'Agent deployed and service started successfully.'
            }}
          </p>
          <p
            v-if="deployResult.available_version"
            class="deploy-version-info"
          >
            {{
              isRedeploy
                ? `${hostname} is running the reinstalled agent and has reconnected.`
                : isUpgrade
                  ? `${hostname} is running the new agent and has reconnected.`
                  : `Deployed version: ${deployResult.available_version}`
            }}
          </p>
          <p class="token-warning">A new agent token was generated for this deployment:</p>
          <div class="token-box">
            <code class="token-text">{{ deployResult.token }}</code>
          </div>
        </template>
      </div>
    </template>

    <template #footer>
      <template v-if="!deployResult?.success">
        <button
          class="btn btn-ghost"
          @click="emit('close')"
        >
          Cancel
        </button>
        <button
          class="btn btn-primary"
          :disabled="deployLoading || !deployForm.ssh_host || !deployForm.server_url"
          @click="submitDeploy"
        >
          {{ submitLabel() }}
        </button>
      </template>
      <template v-else>
        <button
          class="btn btn-primary"
          @click="emit('close')"
        >
          Done
        </button>
      </template>
    </template>
  </BaseModal>
</template>

<style scoped>
.deploy-info {
  font-size: var(--fs-base);
  color: var(--text-muted);
  margin-bottom: var(--space-4);
}

.deploy-note {
  font-size: var(--fs-sm);
  color: var(--text-muted);
  background: var(--bg-input);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: var(--space-4) var(--space-5);
  margin-bottom: var(--space-6);
}

.deploy-note code {
  font-size: var(--fs-xs);
  background: var(--bg-card);
  padding: var(--space-1) var(--space-2);
  border-radius: var(--radius-sm);
}

.upgrade-hero {
  display: flex;
  align-items: center;
  gap: var(--space-6);
  background: var(--accent-subtle);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: var(--space-6) var(--space-7);
  margin-bottom: var(--space-5);
}

.hero-version {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  min-width: 0;
}

.hero-version .value {
  font-size: var(--fs-md);
  font-weight: 600;
  color: var(--text-secondary);
}

.hero-version .value--next {
  color: var(--accent);
}

.hero-arrow {
  color: var(--text-muted);
  display: flex;
  flex: none;
}

.hero-note {
  font-size: var(--fs-sm);
  color: var(--text-muted);
}

.icon-list code {
  font-size: var(--fs-xs);
  background: var(--bg-card);
  padding: var(--space-1) var(--space-2);
  border-radius: var(--radius-sm);
}

.field-label-row {
  margin-bottom: var(--space-2);
}

.field-label-row .field-label {
  margin-bottom: 0;
}

.disclosure-actions {
  display: flex;
  justify-content: flex-end;
  margin-bottom: var(--space-3);
}

.service-textarea {
  font-size: var(--fs-xs);
  line-height: 1.5;
  resize: vertical;
  min-height: 180px;
  white-space: pre;
  overflow-x: auto;
}

.deploy-success-msg {
  color: var(--success);
  font-weight: 600;
  margin-bottom: var(--space-4);
}

.deploy-skipped-msg {
  color: var(--text-secondary);
  font-weight: 500;
}

.deploy-version-info {
  font-size: var(--fs-base);
  color: var(--text-muted);
  font-family: var(--mono);
}
</style>
