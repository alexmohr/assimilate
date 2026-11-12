// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { ref, type Ref } from 'vue'
import { getVapidPublicKey, subscribePush } from '../api/notifications'

/**
 * This browser's web push subscription. Creating a web push channel and
 * sending one a test both need it, so the Notifications page and a channel's
 * own page share this rather than each carrying a copy.
 */
export interface PushSubscriptionState {
  /** Whether this browser can receive web push at all. */
  isPushSupported: () => boolean
  /** The subscription this browser holds, once one is known. */
  subscription: Ref<PushSubscription | null>
  /**
   * Asks for notification permission if needed and (re)subscribes this
   * browser with the server's VAPID key. Throws with a message for the user
   * when it cannot.
   */
  ensurePushSubscription: () => Promise<void>
}

function urlBase64ToUint8Array(base64String: string): Uint8Array<ArrayBuffer> {
  const padding = '='.repeat((4 - (base64String.length % 4)) % 4)
  const base64 = (base64String + padding).replace(/-/g, '+').replace(/_/g, '/')
  const rawData = window.atob(base64)
  const outputArray = new Uint8Array(rawData.length)
  for (let i = 0; i < rawData.length; ++i) {
    outputArray[i] = rawData.charCodeAt(i)
  }
  return outputArray
}

export function usePushSubscription(): PushSubscriptionState {
  const subscription = ref<PushSubscription | null>(null)

  function isPushSupported(): boolean {
    return 'serviceWorker' in navigator && 'PushManager' in window
  }

  async function ensurePushSubscription(): Promise<void> {
    if (!isPushSupported()) {
      throw new Error('Push notifications are not supported in this browser')
    }

    if (Notification.permission === 'denied') {
      throw new Error('Notification permission was denied. Please enable it in browser settings.')
    }

    if (Notification.permission !== 'granted') {
      let result = await Notification.requestPermission()
      if (result === 'default') {
        result = await Notification.requestPermission()
      }
      if (result !== 'granted') {
        throw new Error('Notification permission is required for web push')
      }
    }

    const vapidStatus = await getVapidPublicKey()
    if (!vapidStatus.configured) {
      throw new Error('VAPID keys not configured on the server')
    }

    const registration = await navigator.serviceWorker.ready

    const existing = await registration.pushManager.getSubscription()
    if (existing) {
      await existing.unsubscribe()
    }

    const created = await registration.pushManager.subscribe({
      userVisibleOnly: true,
      applicationServerKey: urlBase64ToUint8Array(vapidStatus.key),
    })
    await subscribePush(created.toJSON())
    subscription.value = created
  }

  return { isPushSupported, subscription, ensurePushSubscription }
}
