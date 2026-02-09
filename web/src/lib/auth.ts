const TOKEN_KEY = 'coop_token'

/** Extract token from URL ?token= param, store in sessionStorage, and strip from URL. */
export function initToken(): string | null {
  // Check URL param first
  const params = new URLSearchParams(window.location.search)
  const urlToken = params.get('token')

  if (urlToken) {
    sessionStorage.setItem(TOKEN_KEY, urlToken)
    // Strip token from URL to avoid leaking in history/referrer
    params.delete('token')
    const newSearch = params.toString()
    const newUrl = window.location.pathname + (newSearch ? `?${newSearch}` : '') + window.location.hash
    window.history.replaceState(null, '', newUrl)
    return urlToken
  }

  // Fall back to sessionStorage
  return sessionStorage.getItem(TOKEN_KEY)
}

export function getToken(): string | null {
  return sessionStorage.getItem(TOKEN_KEY)
}
