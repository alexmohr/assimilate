// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type {
  ChannelConfig,
  ChannelConfigInput,
  ChannelType,
  EmailConfigInput,
  WebhookConfig,
} from '../types/generated'

/**
 * The configuration to submit for a transport, taken from the form draft for
 * that transport. A web push channel has nothing to fill in here: which
 * user's devices it pushes to is decided by the server.
 */
export function configInputFor(
  channelType: ChannelType,
  drafts: { email: EmailConfigInput; webhook: WebhookConfig },
): ChannelConfigInput {
  switch (channelType) {
    case 'email':
      return { channel_type: 'email', config: drafts.email }
    case 'webhook':
      return { channel_type: 'webhook', config: drafts.webhook }
    case 'web_push':
      return { channel_type: 'web_push', config: {} }
  }
}

/**
 * A channel's current configuration with new content templates, ready to
 * submit. Everything else about the configuration is kept as it is.
 */
export function withTemplates(
  current: ChannelConfig,
  title_template: string,
  body_template: string,
): ChannelConfigInput {
  switch (current.channel_type) {
    case 'email':
      return {
        channel_type: 'email',
        config: { ...current.config, title_template, body_template },
      }
    case 'webhook':
      return {
        channel_type: 'webhook',
        config: { ...current.config, title_template, body_template },
      }
    case 'web_push':
      return { channel_type: 'web_push', config: { title_template, body_template } }
  }
}
