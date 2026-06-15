<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, reactive, onMounted } from 'vue'
import { apiClient } from '../api/client'
import { useEscapeKey } from '../composables/useEscapeKey'
import { extractError } from '../utils/error'
import ToggleSwitch from './ToggleSwitch.vue'

const props = defineProps<{
  hostname: string
  agentVersion: string | null
}>()

const emit = defineEmits<{
  close: []
  deployed: [version: string | undefined]
}>()

const visible = ref(true)

useEscapeKey(visible, () => {
  emit('close')
})

const deployLoading = ref(false)
const deployError = ref<string | null>(null)
const fetchServiceLoading = ref(false)
const fetchServiceError = ref<string | null>(null)
const deployResult = ref<{
  success: boolean
  skipped: boolean
  token?: string
  available_version?: string
  error?: string
} | null>(null)

const deployForm = reactive({
  ssh_host: '',
  ssh_user: 'root',
  ssh_port: 22,
  ssh_password: '',
  server_url: '',
  install_path: '/usr/local/bin/assimilate-agent',
  use_sudo: false,
  sudo_password: '',
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

onMounted(() => {
  deployForm.ssh_host = props.hostname
  deployForm.server_url = window.location.origin
  deployForm.systemd_service_content = defaultSystemdUnit('/usr/local/bin/assimilate-agent')
})

async function loadExistingServiceUnit(): Promise<void> {
  if (!deployForm.ssh_host) return
  fetchServiceLoading.value = true
  fetchServiceError.value = null
  try {
    const res = await apiClient.post<{ content: string | null }>(
      `/agents/${props.hostname}/service-unit`,
      {
        ssh_host: deployForm.ssh_host.trim(),
        ssh_user: deployForm.ssh_user.trim(),
        ssh_port: deployForm.ssh_port,
        ssh_password: deployForm.ssh_password || undefined,
        use_sudo: deployForm.use_sudo,
        sudo_password:
          deployForm.use_sudo && deployForm.sudo_password ? deployForm.sudo_password : undefined,
      },
    )
    if (res.data.content) {
      deployForm.systemd_service_content = res.data.content
    } else {
      fetchServiceError.value = 'No existing service unit found on remote host.'
    }
  } catch (e: unknown) {
    fetchServiceError.value = extractError(e)
  } finally {
    fetchServiceLoading.value = false
  }
}

function dialogTitle(): string {
  return props.agentVersion ? 'Upgrade' : 'Deploy'
}

function submitLabel(): string {
  if (deployLoading.value) return 'Deploying...'
  return props.agentVersion ? 'Upgrade Agent' : 'Deploy Agent'
}

async function submitDeploy(): Promise<void> {
  deployLoading.value = true
  deployError.value = null
  deployResult.value = null
  try {
    const res = await apiClient.post<{
      success: boolean
      skipped: boolean
      token?: string
      available_version?: string
      error?: string
    }>(`/agents/${props.hostname}/deploy`, {
      ssh_host: deployForm.ssh_host.trim(),
      ssh_user: deployForm.ssh_user.trim(),
      ssh_port: deployForm.ssh_port,
      ssh_password: deployForm.ssh_password || undefined,
      server_url: deployForm.server_url.trim(),
      install_path: deployForm.install_path.trim() || undefined,
      use_sudo: deployForm.use_sudo,
      sudo_password:
        deployForm.use_sudo && deployForm.sudo_password ? deployForm.sudo_password : undefined,
      systemd_service_content: deployForm.systemd_service_content.trim() || undefined,
    })
    deployResult.value = res.data
    if (res.data.success) {
      emit('deployed', res.data.available_version)
    }
  } catch (e: unknown) {
    deployError.value = extractError(e)
  } finally {
    deployLoading.value = false
  }
}
</script>

<template>
  <Teleport to="body">
    <div
      class="overlay"
      @click.self="emit('close')"
    >
      <div class="dialog">
        <div class="dialog-header">
          <h2 class="dialog-title">
            {{ dialogTitle() }} Agent &mdash;
            {{ hostname }}
          </h2>
          <button
            class="close-btn"
            @click="emit('close')"
          >
            &times;
          </button>
        </div>

        <template v-if="!deployResult?.success">
          <div class="dialog-body">
            <p class="deploy-info">
              Upload and install the agent binary on the target machine via SSH. Connect as root or
              enable sudo below for non-root users.
            </p>
            <p class="deploy-note">
              This will also install and enable the <code>assimilate-agent</code> systemd service on
              the target machine. You can customize the service unit below.
            </p>
            <div class="field">
              <label class="field-label">SSH Host <span class="required">*</span></label>
              <input
                v-model="deployForm.ssh_host"
                class="input mono"
                placeholder="e.g. 192.168.1.10"
              />
            </div>
            <div class="deploy-row-fields">
              <div class="field">
                <label class="field-label">SSH User</label>
                <input
                  v-model="deployForm.ssh_user"
                  class="input mono"
                  placeholder="root"
                />
              </div>
              <div class="field field-narrow">
                <label class="field-label">SSH Port</label>
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
              <label class="field-label">SSH Password</label>
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
              <label class="field-label">Install Path</label>
              <input
                v-model="deployForm.install_path"
                class="input mono"
                placeholder="/usr/local/bin/assimilate-agent"
              />
            </div>
            <div class="field toggle-row">
              <span class="toggle-row-label">Use sudo for privileged operations</span>
              <ToggleSwitch v-model="deployForm.use_sudo" />
            </div>
            <span
              v-if="deployForm.use_sudo"
              class="field-hint"
              >Enable when connecting as a non-root user that has sudo access</span
            >
            <div
              v-if="deployForm.use_sudo"
              class="field"
            >
              <label class="field-label">Sudo Password</label>
              <input
                v-model="deployForm.sudo_password"
                class="input mono"
                type="password"
                placeholder="Leave empty if passwordless sudo is configured"
              />
            </div>
            <div class="field">
              <div class="field-label-row">
                <label class="field-label">Systemd Service Unit</label>
                <button
                  class="btn btn-sm btn-ghost"
                  type="button"
                  :disabled="fetchServiceLoading || !deployForm.ssh_host"
                  @click="loadExistingServiceUnit"
                >
                  {{ fetchServiceLoading ? 'Loading…' : 'Load from remote' }}
                </button>
              </div>
              <textarea
                v-model="deployForm.systemd_service_content"
                class="input mono service-textarea"
                rows="12"
                spellcheck="false"
              />
              <span class="field-hint">
                The <code>BORG_SERVER_URL</code> and <code>BORG_AGENT_TOKEN</code> environment
                variables will be injected automatically if not present in custom content.
              </span>
              <span
                v-if="fetchServiceError"
                class="field-hint field-hint-error"
              >
                {{ fetchServiceError }}
              </span>
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
          </div>
          <div class="dialog-footer">
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
          </div>
        </template>

        <template v-else>
          <div class="dialog-body">
            <div class="token-notice">
              <template v-if="deployResult.skipped">
                <p class="deploy-skipped-msg">
                  Agent is already at the latest version ({{ deployResult.available_version }}).
                  Deployment skipped.
                </p>
              </template>
              <template v-else>
                <p class="deploy-success-msg">Agent deployed and service started successfully.</p>
                <p
                  v-if="deployResult.available_version"
                  class="deploy-version-info"
                >
                  Deployed version: {{ deployResult.available_version }}
                </p>
                <p class="token-warning">A new agent token was generated for this deployment:</p>
                <div class="token-box">
                  <code class="token-text">{{ deployResult.token }}</code>
                </div>
              </template>
            </div>
          </div>
          <div class="dialog-footer">
            <button
              class="btn btn-primary"
              @click="emit('close')"
            >
              Done
            </button>
          </div>
        </template>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.deploy-info {
  font-size: 0.85rem;
  color: var(--text-muted);
  margin-bottom: 0.5rem;
}

.deploy-note {
  font-size: 0.8rem;
  color: var(--text-muted);
  background: var(--bg-input);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 0.5rem 0.75rem;
  margin-bottom: 1rem;
}

.deploy-note code {
  font-size: 0.75rem;
  background: var(--bg-card);
  padding: 0.1rem 0.3rem;
  border-radius: 3px;
}

.deploy-row-fields {
  display: flex;
  gap: 1rem;
}

.deploy-row-fields .field {
  flex: 1;
}

.field-narrow {
  max-width: 120px;
  flex: 0 0 120px;
}

.toggle-row {
  display: flex;
  flex-direction: row;
  gap: 1.5rem;
  align-items: center;
  margin-top: 0.5rem;
}

.toggle-row-label {
  font-size: 0.875rem;
  color: var(--text-secondary);
}

.field-label-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 0.3rem;
}

.field-label-row .field-label {
  margin-bottom: 0;
}

.field-hint-error {
  color: var(--danger);
}

.service-textarea {
  font-size: 0.75rem;
  line-height: 1.5;
  resize: vertical;
  min-height: 180px;
  white-space: pre;
  overflow-x: auto;
}

.deploy-success-msg {
  color: var(--success);
  font-weight: 600;
  margin-bottom: 0.5rem;
}

.deploy-skipped-msg {
  color: var(--text-secondary);
  font-weight: 500;
}

.deploy-version-info {
  font-size: 0.85rem;
  color: var(--text-muted);
  font-family: var(--mono);
}

.token-notice {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}

.token-warning {
  color: var(--warning);
  font-size: 0.875rem;
  font-weight: 500;
}

.token-box {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  background: var(--bg-input);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 0.75rem 1rem;
}

.token-text {
  flex: 1;
  font-family: var(--mono);
  font-size: 0.78rem;
  color: var(--success);
  word-break: break-all;
  background: transparent;
  padding: 0;
}
</style>
