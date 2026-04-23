// Bot-HQ Live — WebSocket client with full audio capture and playback

(function () {
    "use strict";

    // ── Constants ──────────────────────────────────────────────────────
    var WS_URL = "ws://" + window.location.host + "/ws";
    var CAPTURE_SAMPLE_RATE = 16000;
    var PLAYBACK_SAMPLE_RATE = 24000;

    // ── State ──────────────────────────────────────────────────────────
    var ws = null;
    var micStream = null;
    var isCapturing = false;
    var captureContext = null;
    var captureSource = null;
    var workletNode = null;
    var scriptProcessor = null;

    // Playback state
    var playbackContext = null;
    var playbackNextTime = 0;
    var playbackSources = [];
    var isSpeaking = false;

    // DOM
    var statusEl = document.getElementById("status");
    var micBtn = document.getElementById("mic-btn");
    var transcriptEl = document.getElementById("transcript");

    // ── Status ──────────────────────────────────────────────────────────
    function updateStatus(cls, text) {
        statusEl.className = "status " + cls;
        statusEl.textContent = text;
    }

    // ── WebSocket ──────────────────────────────────────────────────────
    function connect() {
        updateStatus("connecting", "Connecting...");
        ws = new WebSocket(WS_URL);

        ws.onopen = function () {
            updateStatus("connected", "Connected");
            micBtn.disabled = false;
        };

        ws.onclose = function () {
            updateStatus("disconnected", "Disconnected");
            micBtn.disabled = true;
            stopCapture();
            setTimeout(connect, 2000);
        };

        ws.onerror = function () {
            updateStatus("error", "Connection Error");
        };

        ws.onmessage = function (event) {
            try {
                var msg = JSON.parse(event.data);
                handleMessage(msg);
            } catch (e) {
                console.error("Failed to parse message:", e);
            }
        };
    }

    function handleMessage(msg) {
        switch (msg.type) {
            case "connected":
                updateStatus("connected", "Connected");
                break;
            case "audio":
                if (!isSpeaking) {
                    isSpeaking = true;
                    updateStatus("speaking", "Speaking...");
                }
                enqueuePlayback(msg.data);
                break;
            case "transcript":
                addTranscript(msg.role, msg.text);
                break;
            case "turn_complete":
                isSpeaking = false;
                if (isCapturing) {
                    updateStatus("listening", "Listening...");
                } else {
                    updateStatus("connected", "Connected");
                }
                break;
            case "interrupted":
                stopPlayback();
                isSpeaking = false;
                if (isCapturing) {
                    updateStatus("listening", "Listening...");
                } else {
                    updateStatus("connected", "Connected");
                }
                break;
        }
    }

    // ── Transcript ─────────────────────────────────────────────────────
    function addTranscript(role, text) {
        if (!text) return;
        var div = document.createElement("div");
        div.className = "transcript-entry " + role;

        var label = document.createElement("span");
        label.className = "transcript-role";
        label.textContent = role === "user" ? "You" : "Assistant";

        var content = document.createElement("span");
        content.className = "transcript-text";
        content.textContent = text;

        div.appendChild(label);
        div.appendChild(content);
        transcriptEl.appendChild(div);
        transcriptEl.scrollTop = transcriptEl.scrollHeight;
    }

    // ── Mic Button ─────────────────────────────────────────────────────
    micBtn.addEventListener("click", function () {
        if (isCapturing) {
            stopCapture();
        } else {
            startCapture();
        }
    });

    // ── Audio Capture (Mic → WebSocket) ────────────────────────────────
    function float32ToInt16Base64(float32) {
        var int16 = new Int16Array(float32.length);
        for (var i = 0; i < float32.length; i++) {
            var s = Math.max(-1, Math.min(1, float32[i]));
            int16[i] = s < 0 ? s * 0x8000 : s * 0x7fff;
        }
        var bytes = new Uint8Array(int16.buffer);
        var binary = "";
        for (var j = 0; j < bytes.length; j++) {
            binary += String.fromCharCode(bytes[j]);
        }
        return btoa(binary);
    }

    function sendAudioChunk(float32Data) {
        if (!ws || ws.readyState !== WebSocket.OPEN) return;
        var base64 = float32ToInt16Base64(float32Data);
        ws.send(JSON.stringify({ type: "audio", data: base64 }));
    }

    async function startCapture() {
        try {
            var stream = await navigator.mediaDevices.getUserMedia({
                audio: {
                    sampleRate: CAPTURE_SAMPLE_RATE,
                    channelCount: 1,
                    echoCancellation: true,
                    noiseSuppression: true,
                    autoGainControl: true,
                },
            });
            micStream = stream;
            isCapturing = true;
            micBtn.classList.add("active");
            updateStatus("listening", "Listening...");

            captureContext = new AudioContext({ sampleRate: CAPTURE_SAMPLE_RATE });
            captureSource = captureContext.createMediaStreamSource(stream);

            // Try AudioWorklet first, fall back to ScriptProcessorNode
            var useWorklet = typeof AudioWorkletNode !== "undefined";
            if (useWorklet) {
                try {
                    await setupWorkletCapture();
                } catch (e) {
                    console.warn("AudioWorklet failed, falling back to ScriptProcessor:", e);
                    setupScriptProcessorCapture();
                }
            } else {
                setupScriptProcessorCapture();
            }
        } catch (err) {
            console.error("Mic access denied:", err);
            updateStatus("error", "Mic access denied");
        }
    }

    async function setupWorkletCapture() {
        var processorCode = [
            "class PCMProcessor extends AudioWorkletProcessor {",
            "    constructor() {",
            "        super();",
            "        this.buffer = new Float32Array(0);",
            "        this.bufferSize = 1600;", // 100ms at 16kHz
            "    }",
            "    process(inputs) {",
            "        const input = inputs[0][0];",
            "        if (!input) return true;",
            "        const newBuf = new Float32Array(this.buffer.length + input.length);",
            "        newBuf.set(this.buffer);",
            "        newBuf.set(input, this.buffer.length);",
            "        this.buffer = newBuf;",
            "        while (this.buffer.length >= this.bufferSize) {",
            "            const chunk = this.buffer.slice(0, this.bufferSize);",
            "            this.port.postMessage(chunk);",
            "            this.buffer = this.buffer.slice(this.bufferSize);",
            "        }",
            "        return true;",
            "    }",
            "}",
            "registerProcessor('pcm-processor', PCMProcessor);",
        ].join("\n");

        var blob = new Blob([processorCode], { type: "application/javascript" });
        var url = URL.createObjectURL(blob);
        await captureContext.audioWorklet.addModule(url);
        URL.revokeObjectURL(url);

        workletNode = new AudioWorkletNode(captureContext, "pcm-processor");
        workletNode.port.onmessage = function (event) {
            sendAudioChunk(event.data);
        };
        captureSource.connect(workletNode);
        workletNode.connect(captureContext.destination);
    }

    function setupScriptProcessorCapture() {
        scriptProcessor = captureContext.createScriptProcessor(2048, 1, 1);
        var buffer = new Float32Array(0);
        var bufferSize = 1600; // 100ms at 16kHz

        scriptProcessor.onaudioprocess = function (e) {
            var input = e.inputBuffer.getChannelData(0);

            // Accumulate into buffer
            var newBuf = new Float32Array(buffer.length + input.length);
            newBuf.set(buffer);
            newBuf.set(input, buffer.length);
            buffer = newBuf;

            // Send complete chunks
            while (buffer.length >= bufferSize) {
                var chunk = buffer.slice(0, bufferSize);
                sendAudioChunk(chunk);
                buffer = buffer.slice(bufferSize);
            }
        };

        captureSource.connect(scriptProcessor);
        scriptProcessor.connect(captureContext.destination);
    }

    function stopCapture() {
        if (workletNode) {
            workletNode.disconnect();
            workletNode = null;
        }
        if (scriptProcessor) {
            scriptProcessor.disconnect();
            scriptProcessor = null;
        }
        if (captureSource) {
            captureSource.disconnect();
            captureSource = null;
        }
        if (captureContext) {
            captureContext.close().catch(function () {});
            captureContext = null;
        }
        if (micStream) {
            micStream.getTracks().forEach(function (t) {
                t.stop();
            });
            micStream = null;
        }
        isCapturing = false;
        micBtn.classList.remove("active");
        if (!isSpeaking) {
            if (ws && ws.readyState === WebSocket.OPEN) {
                updateStatus("connected", "Connected");
            }
        }
    }

    // ── Audio Playback (WebSocket → Speaker) ───────────────────────────
    function initPlaybackContext() {
        if (!playbackContext) {
            playbackContext = new AudioContext({ sampleRate: PLAYBACK_SAMPLE_RATE });
        }
        if (playbackContext.state === "suspended") {
            playbackContext.resume();
        }
    }

    function enqueuePlayback(base64PCM) {
        initPlaybackContext();
        var ctx = playbackContext;

        // Decode base64 → Int16 → Float32
        var raw = atob(base64PCM);
        var bytes = new Uint8Array(raw.length);
        for (var i = 0; i < raw.length; i++) {
            bytes[i] = raw.charCodeAt(i);
        }
        var int16 = new Int16Array(bytes.buffer);
        var float32 = new Float32Array(int16.length);
        for (var j = 0; j < int16.length; j++) {
            float32[j] = int16[j] / 0x7fff;
        }

        // Create audio buffer
        var audioBuffer = ctx.createBuffer(1, float32.length, PLAYBACK_SAMPLE_RATE);
        audioBuffer.copyToChannel(float32, 0);

        // Schedule gapless playback
        var source = ctx.createBufferSource();
        source.buffer = audioBuffer;
        source.connect(ctx.destination);

        var now = ctx.currentTime;
        var startAt = Math.max(now, playbackNextTime);
        source.start(startAt);
        playbackNextTime = startAt + audioBuffer.duration;

        // Track for cleanup
        playbackSources.push(source);
        source.onended = function () {
            playbackSources = playbackSources.filter(function (s) {
                return s !== source;
            });
            if (playbackSources.length === 0 && isSpeaking) {
                // All queued audio finished playing
                isSpeaking = false;
                if (isCapturing) {
                    updateStatus("listening", "Listening...");
                } else if (ws && ws.readyState === WebSocket.OPEN) {
                    updateStatus("connected", "Connected");
                }
            }
        };
    }

    function stopPlayback() {
        for (var i = 0; i < playbackSources.length; i++) {
            try {
                playbackSources[i].onended = null;
                playbackSources[i].stop();
            } catch (e) {
                /* already stopped */
            }
        }
        playbackSources = [];
        playbackNextTime = 0;
        isSpeaking = false;
    }

    // ── Init ───────────────────────────────────────────────────────────
    // Ensure playback context is created on first user interaction
    // (browsers require user gesture to create AudioContext)
    document.addEventListener(
        "click",
        function () {
            initPlaybackContext();
        },
        { once: true }
    );

    connect();
})();
