(function (g) {
  class Player {
    constructor(video, options) {
      this.video = video;
      this.options = options || {};
      this.pc = null;
      this.flvPlayer = null;
      this.hls = null;
      this.onError = this.options.onError || null;
      this.onTrack = this.options.onTrack || null;
      this.onReconnect = this.options.onReconnect || null;
      this.onLevel = this.options.onLevel || null;
      this.levels = this.options.levels || [];
      this.currentLevel = -1;
      this.retries = 0;
      this.closed = false;
      this.reconnectTimer = null;
      this.abrTimer = null;
      this.flvAbrTimer = null;
      this._lastBytes = null;
      this.current = null;
      this.retry = Object.assign({ max: 5, delay: 1000, factor: 2 }, this.options.retry || {});
      this._nativeError = null;
    }

    async play(url, type) {
      this.closed = false;
      this.retries = 0;
      if (this.levels.length && (!url || this.currentLevel < 0)) {
        const idx = this.options.autoLevel === false ? 0 : this.autoLevel();
        this.currentLevel = idx;
        url = this.levels[idx].url;
      }
      this.current = { url, type };
      return this.start();
    }

    async start() {
      if (!this.current) throw new Error("no current source");
      const { url, type } = this.current;
      this.clearReconnect();
      this.cleanup();
      try {
        if (type === "whep" || type === "webrtc") return await this.whep(url);
        if (type === "flv" && g.flvjs) return this.flv(url);
        return await this.native(url);
      } catch (e) {
        this.fail(e);
        throw e;
      }
    }

    native(url) {
      this.video.src = url;
      if (this.video.addEventListener) {
        this._nativeError = () => this.scheduleReconnect("media-error");
        this.video.addEventListener("error", this._nativeError);
      }
      return Promise.resolve(this.video.play && this.video.play());
    }

    hlsjs(url) {
      const hls = new g.Hls(this.options.hls || {});
      this.hls = hls;
      if (hls.on && g.Hls.Events) {
        hls.on(g.Hls.Events.MANIFEST_PARSED, (_, data) => {
          if (data && data.levels) {
            this.levels = data.levels.map((l, i) => ({ bitrate: l.bitrate || 0, url: this.current ? this.current.url : "", index: i }));
          }
        });
        hls.on(g.Hls.Events.ERROR, (_, data) => {
          if (data && data.fatal) this.scheduleReconnect("hls-error");
        });
      }
      hls.attachMedia(this.video);
      hls.loadSource(url);
      return Promise.resolve(hls.play && hls.play());
    }

    flv(url) {
      this.flvPlayer = g.flvjs.createPlayer({ type: "flv", url });
      if (this.flvPlayer.on && g.flvjs.Events) {
        this.flvPlayer.on(g.flvjs.Events.ERROR, () => this.scheduleReconnect("flv-error"));
      }
      this.flvPlayer.attachMediaElement(this.video);
      this.flvPlayer.load();
      return Promise.resolve(this.flvPlayer.play());
    }

    startFlvAbr() {
      if (this.flvAbrTimer || this.levels.length < 2 || this.options.autoLevel === false) return;
      this.flvAbrTimer = setInterval(() => {
        const s = this.flvPlayer && this.flvPlayer.statisticsInfo;
        const speed = s && s.speed;
        if (speed > 0) this.flvAbrTick(speed * 8);
      }, 5000);
    }

    flvAbrTick(kbps) {
      const idx = this.chooseLevel(kbps * 1000);
      if (idx < 0 || idx === this.currentLevel) return false;
      this.setLevel(idx);
      return true;
    }

    async whep(url) {
      const pc = new RTCPeerConnection(this.options.rtc || {});
      this.pc = pc;
      pc.addTransceiver("video", { direction: "recvonly" });
      pc.addTransceiver("audio", { direction: "recvonly" });
      pc.ontrack = (e) => {
        if (e.track.kind === "video") this.video.srcObject = e.streams[0];
        if (this.onTrack) this.onTrack(e);
      };
      pc.onconnectionstatechange = () => {
        if (this.closed) return;
        if (pc.connectionState === "failed" || pc.connectionState === "disconnected") {
          this.scheduleReconnect("ice-" + pc.connectionState);
        }
      };
      const offer = await pc.createOffer();
      await pc.setLocalDescription(offer);
      await new Promise((r) => {
        if (pc.iceGatheringState === "complete") return r();
        pc.onicegatheringstatechange = () => {
          if (pc.iceGatheringState === "complete") r();
        };
        setTimeout(r, 3000);
      });
      const headers = { "Content-Type": "application/sdp" };
      if (this.options.token) headers.Authorization = "Bearer " + this.options.token;
      const res = await fetch(url, {
        method: "POST",
        headers,
        body: pc.localDescription.sdp,
      });
      if (!res.ok) throw new Error("WHEP " + res.status);
      await pc.setRemoteDescription({ type: "answer", sdp: await res.text() });
      this.startAbr();
    }

    scheduleReconnect(reason) {
      if (this.closed || this.reconnectTimer) return;
      if (this.retries >= this.retry.max) {
        this.fail(new Error("reconnect exhausted: " + reason));
        return;
      }
      const delay = this.retry.delay * Math.pow(this.retry.factor, this.retries);
      this.reconnectTimer = setTimeout(async () => {
        this.reconnectTimer = null;
        this.retries += 1;
        if (this.onReconnect) this.onReconnect({ attempt: this.retries, reason });
        try {
          await this.start();
        } catch (e) {
          this.scheduleReconnect("retry-error");
        }
      }, delay);
    }

    async reconnect() {
      this.closed = false;
      this.retries = 0;
      return this.start();
    }

    chooseLevel(kbps) {
      if (!this.levels.length) return -1;
      let idx = -1;
      this.levels.forEach((l, i) => { if ((l.bitrate || 0) <= kbps) idx = i; });
      if (idx < 0) idx = 0;
      return idx;
    }

    startAbr() {
      if (this.abrTimer || this.levels.length < 2 || this.options.autoLevel === false) return;
      this.abrTimer = setInterval(async () => {
        if (this.closed || !this.pc) return;
        try {
          const stats = await this.pc.getStats();
          let bytes = 0;
          stats.forEach((r) => {
            if (r.type === "inbound-rtp" && r.kind === "video") bytes = r.bytesReceived || 0;
          });
          if (this._lastBytes != null && bytes >= this._lastBytes) {
            const kbps = ((bytes - this._lastBytes) * 8) / 5;
            const idx = this.chooseLevel(kbps * 1000);
            if (idx >= 0 && idx !== this.currentLevel) this.setLevel(idx);
          }
          this._lastBytes = bytes;
        } catch (_) {}
      }, 5000);
    }

    autoLevel() {
      if (!this.levels.length) return -1;
      let downlink = 2_000_000;
      if (g.navigator && g.navigator.connection && g.navigator.connection.downlink) {
        downlink = g.navigator.connection.downlink * 1000 * 1000;
      }
      let idx = this.levels.findIndex((l) => (l.bitrate || 0) <= downlink);
      if (idx < 0) idx = this.levels.length - 1;
      return idx;
    }

    setLevel(index) {
      if (index < 0 || index >= this.levels.length) return false;
      this.currentLevel = index;
      const type = this.current ? this.current.type : this.options.type || "hls";
      this.current = { url: this.levels[index].url, type };
      if (this.onLevel) this.onLevel({ index, level: this.levels[index] });
      if (!this.closed) this.start().catch((e) => this.fail(e));
      return true;
    }

    async stats() {
      if (!this.pc) return null;
      return await this.pc.getStats();
    }

    fail(e) {
      if (this.onError) this.onError(e);
    }

    clearReconnect() {
      if (this.reconnectTimer) {
        clearTimeout(this.reconnectTimer);
        this.reconnectTimer = null;
      }
    }

    cleanup() {
      if (this.abrTimer) { clearInterval(this.abrTimer); this.abrTimer = null; }
      if (this.flvAbrTimer) { clearInterval(this.flvAbrTimer); this.flvAbrTimer = null; }
      this._lastBytes = null;
      if (this.flvPlayer) {
        if (this.flvPlayer.destroy) this.flvPlayer.destroy();
        this.flvPlayer = null;
      this.hls = null;
      }
      if (this.pc) {
        this.pc.close();
        this.pc = null;
      }
    }

    close() {
      this.closed = true;
      this.clearReconnect();
      this.cleanup();
      if (this._nativeError && this.video.removeEventListener) {
        this.video.removeEventListener("error", this._nativeError);
      }
      this._nativeError = null;
      if (this.video) this.video.srcObject = null;
    }
  }

  g.M7S = Object.assign(g.M7S || {}, {
    Player,
    createPlayer: (v, o) => new Player(v, o),
    version: "1.1.0",
  });
})(typeof window !== "undefined" ? window : globalThis);
