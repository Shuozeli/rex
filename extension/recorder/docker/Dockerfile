# A desktop for recording the extension on real pages: Xvfb + openbox + Google Chrome (stable, branded) with
# the extension force-installed by enterprise policy from a packed CRX, ffmpeg (x11grab) and xdotool for real
# capture and input, x11vnc + noVNC to watch live.
FROM ubuntu:24.04

ARG NODE_VERSION=22.22.2
ENV DEBIAN_FRONTEND=noninteractive TZ=UTC LANG=C.UTF-8

RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates curl unzip xz-utils tini procps openssl libnss3-tools libcap2-bin \
      xvfb openbox x11-utils x11-xserver-utils xdotool imagemagick ffmpeg \
      x11vnc novnc websockify \
      python3 python3-numpy python3-pil \
      fonts-liberation fonts-noto-core fonts-noto-cjk fonts-noto-color-emoji \
      libasound2t64 libatk-bridge2.0-0t64 libatk1.0-0t64 libatspi2.0-0t64 libcairo2 libcups2t64 libcurl4t64 \
      libdbus-1-3 libexpat1 libgbm1 libglib2.0-0t64 libgtk-3-0t64 libnspr4 libnss3 libpango-1.0-0 libudev1 \
      libvulkan1 libx11-6 libxcb1 libxcomposite1 libxdamage1 libxext6 libxfixes3 libxkbcommon0 libxrandr2 \
      xdg-utils \
    && rm -rf /var/lib/apt/lists/*

# CHROME_REFRESH (CI passes the ISO week) re-runs this layer so a cached image does not keep an old Chrome.
ARG CHROME_REFRESH=
# Google Chrome stable, as users run it. It ignores --load-extension (M137+), so the extension is installed
# the way an organization deploys it: ExtensionInstallForcelist pointing at a local update server.
RUN echo "chrome refresh: ${CHROME_REFRESH}" \
    && curl -fsSL -o /tmp/chrome.deb https://dl.google.com/linux/direct/google-chrome-stable_current_amd64.deb \
    && apt-get update && apt-get install -y --no-install-recommends /tmp/chrome.deb \
    && rm -rf /tmp/chrome.deb /var/lib/apt/lists/* /etc/apt/sources.list.d/google-chrome.list \
    && google-chrome --version

RUN curl -fsSL "https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-linux-x64.tar.xz" \
      | tar -xJ -C /opt && ln -s /opt/node-v${NODE_VERSION}-linux-x64/bin/node /usr/local/bin/node \
    && setcap cap_net_bind_service=+ep /opt/node-v${NODE_VERSION}-linux-x64/bin/node   # fixture server on :443

# Policies: no sign-in prompt or promos in the recording; DevTools (CDP) allowed on the force-installed
# extension, which DeveloperToolsAvailability 0 (the default) refuses ("Not allowed"). The extension policy (rex-extension.json) is written
# at start, once the CRX is packed and its id known; the directory is the desktop user's for that.
RUN mkdir -p /etc/opt/chrome/policies/managed \
    && printf '{"BrowserSignin": 0, "SyncDisabled": true, "PromotionsEnabled": false, "DefaultBrowserSettingEnabled": false, "PasswordManagerEnabled": false, "AutofillAddressEnabled": false, "TranslateEnabled": false, "MetricsReportingEnabled": false, "DeveloperToolsAvailability": 1}\n' \
      > /etc/opt/chrome/policies/managed/rex.json

# uid 1000, like the host user: the signing key (mode 600) is readable and output files are the host user's.
RUN userdel -r ubuntu && useradd -m -u 1000 -s /bin/bash rec && chown rec /etc/opt/chrome/policies/managed
COPY cdp-relay.mjs ext-server.mjs /usr/local/lib/rex/
COPY entrypoint.sh /usr/local/bin/rex-desktop
RUN chmod +x /usr/local/bin/rex-desktop
USER rec
WORKDIR /home/rec

# /ext: the built extension (read-only), /keys/ext.pem: its signing key, /out: recordings.
ENTRYPOINT ["tini", "--", "rex-desktop"]
