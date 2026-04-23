// Bot-HQ Live — WebSocket client + audio capture scaffold

const WS_URL = "ws://" + window.location.host + "/ws";
let ws = null;
let micStream = null;
let isCapturing = false;

function connect() {
    updateStatus("connecting", "Connecting...");
    ws = new WebSocket(WS_URL);

    ws.onopen = function () {
        updateStatus("connected", "Connected");
        document.getElementById("mic-btn").disabled = false;
    };

    ws.onclose = function () {
        updateStatus("disconnected", "Disconnected");
        document.getElementById("mic-btn").disabled = true;
        stopCapture();
        // Reconnect after 2 seconds
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

function updateStatus(cls, text) {
    var el = document.getElementById("status");
    el.className = "status " + cls;
    el.textContent = text;
}

function handleMessage(msg) {
    // Handle different message types from the hub
    if (msg.type === "transcript") {
        addTranscript(msg.role, msg.text);
    } else if (msg.type === "response") {
        addTranscript("assistant", msg.content);
    } else if (msg.type === "update") {
        addTranscript("assistant", msg.content);
    }
}

function addTranscript(role, text) {
    var el = document.getElementById("transcript");
    var div = document.createElement("div");
    div.className = "transcript-entry " + role;
    div.textContent = role + ": " + text;
    el.appendChild(div);
    el.scrollTop = el.scrollHeight;
}

// Mic button toggle
var micBtn = document.getElementById("mic-btn");
micBtn.addEventListener("click", function () {
    if (isCapturing) {
        stopCapture();
    } else {
        startCapture();
    }
});

function startCapture() {
    navigator.mediaDevices
        .getUserMedia({ audio: true })
        .then(function (stream) {
            micStream = stream;
            isCapturing = true;
            micBtn.classList.add("active");
            // Audio processing will be wired in Task 6.3
            console.log("Mic capture started");
        })
        .catch(function (err) {
            console.error("Mic access denied:", err);
        });
}

function stopCapture() {
    if (micStream) {
        micStream.getTracks().forEach(function (t) {
            t.stop();
        });
        micStream = null;
    }
    isCapturing = false;
    micBtn.classList.remove("active");
    console.log("Mic capture stopped");
}

connect();
