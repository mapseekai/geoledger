"""Shared sandboxed Chromium configuration for console CI tests."""


def add_browser_arguments(parser):
    browser = parser.add_mutually_exclusive_group()
    browser.add_argument('--chromium', help='Path to an installed Chromium executable')
    browser.add_argument('--browser-channel', help='Playwright browser channel, e.g. chrome')


def launch_browser(playwright, args):
    return playwright.chromium.launch(
        headless=True,
        executable_path=args.chromium,
        channel=args.browser_channel,
        chromium_sandbox=True,
        args=['--disable-dev-shm-usage'],
    )
