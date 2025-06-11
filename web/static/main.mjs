import init, { compile } from "./playground/wahgex_web_playground.js";

await init();

// Copied from https://codepen.io/wonism/pen/dXgrxE
function debounce(cb, interval, immediate) {
    var timeout;

    return function () {
        const context = this,
            args = arguments;
        const later = function () {
            timeout = null;
            if (!immediate) cb.apply(context, args);
        };

        const callNow = immediate && !timeout;

        clearTimeout(timeout);
        timeout = setTimeout(later, interval);

        if (callNow) cb.apply(context, args);
    };
}

class RegexModule {
    static async create(pattern) {
        if (pattern == null || !(typeof pattern === "string")) {
            return null;
        }

        let moduleBytes;
        try {
            moduleBytes = compile(pattern);
        } catch (err) {
            if (typeof err === "string") {
                throw Error(err);
            } else {
                throw err;
            }
        }
        const { module, instance } = await WebAssembly.instantiate(moduleBytes);

        return new RegexModule(pattern, moduleBytes, module, instance);
    }

    constructor(pattern, moduleBytes, module, instance) {
        this.pattern = pattern;
        this.moduleBytes = moduleBytes;
        this.module = module;
        this.instance = instance;
        this.encoder = new TextEncoder("utf-8");
    }

    isMatch(haystack) {
        if (haystack === null || !(typeof haystack === "string")) {
            return false;
        }

        this.instance.exports.prepare_input(BigInt(haystack.length));
        const view = new Uint8Array(this.instance.exports.haystack.buffer);
        this.encoder.encodeInto(haystack, view);

        // anchored, anchored_pattern, span_start, span_end, haystack_len
        const anchored = 0;
        const anchoredPattern = 0;
        const spanStart = BigInt(0);
        const spanEnd = BigInt(haystack.length);
        const haystackLen = BigInt(haystack.length);
        const result = this.instance.exports.is_match(
            anchored,
            anchoredPattern,
            spanStart,
            spanEnd,
            haystackLen,
        );
        return Boolean(result);
    }
}

function assert(condition, message) {
    if (!condition) {
        throw new Error(message || "Assertion failed");
    }
}

let currentModule = null;
let currentModuleBytes = null;

const searchButton = document.getElementById("searchButton");
assert(searchButton !== null, "expected search button element present");

const downloadWasmButton = document.getElementById("downloadWasmButton");
assert(
    downloadWasmButton !== null,
    "expected download WASM button element present",
);

const regexInput = document.getElementById("regexInput");
assert(regexInput !== null, "expected regex input element present");

const haystackText = document.getElementById("haystackText");
assert(haystackText !== null, "expected haystack text area element present");

const regexErrorDiv = document.getElementById("regexError");
assert(regexErrorDiv !== null, "expected regex error div present");

const searchResultDiv = document.getElementById("searchResult");
assert(searchResultDiv !== null, "expected search result div present");

function clearMessages() {
    regexErrorDiv.textContent = "";
    searchResultDiv.textContent = "";
}

// Helper function to toggle button states and clear module data
function resetModuleState() {
    currentModule = null;
    searchButton.disabled = true;
    downloadWasmButton.disabled = true;
}

function performSearch() {
    searchResultDiv.textContent = "";
    if (currentModule === null) {
        return;
    }

    const haystack = haystackText.value;
    if (haystack === null || !(typeof haystack === "string")) {
        searchResultDiv.textContent = "";
        return;
    }

    const result = currentModule.isMatch(haystack);
    searchResultDiv.textContent = result ? "Match found!" : "No match found.";
}

regexInput.addEventListener(
    "input",
    debounce(function (ev) {
        clearMessages();
        const value = regexInput.value;
        if (
            value === null ||
            !(typeof value === "string") ||
            value.length === 0
        ) {
            resetModuleState();
            searchResultDiv.textContent = ""; // Clear result when regex is cleared
            return;
        }

        RegexModule.create(value)
            .then((module) => {
                currentModule = module;
                searchButton.disabled = false;
                downloadWasmButton.disabled = false;
                // Automatically search if haystack is not empty after successful compilation
                if (haystackText.value.length > 0) {
                    performSearch();
                }
            })
            .catch((err) => {
                resetModuleState();
                regexErrorDiv.textContent = `Compilation error: ${err.message}`;
                searchResultDiv.textContent = ""; // Clear result on compilation error
            });
    }, 500),
);

downloadWasmButton.addEventListener("click", async function () {
    if (currentModule && currentModule.moduleBytes && currentModule.pattern) {
        const blob = new Blob([currentModule.moduleBytes], {
            type: "application/wasm",
        });
        const url = URL.createObjectURL(blob);
        const a = document.createElement("a");
        a.href = url;

        // Generate hash of the regex pattern for the filename
        const pattern = currentModule.pattern;
        const encoder = new TextEncoder();
        const data = encoder.encode(pattern);
        const hashBuffer = await crypto.subtle.digest("SHA-1", data);
        const hashArray = Array.from(new Uint8Array(hashBuffer));
        const hashHex = hashArray
            .map((b) => b.toString(16).padStart(2, "0"))
            .join("");

        a.download = `regex-${hashHex}.wasm`;

        document.body.appendChild(a);
        a.click();
        document.body.removeChild(a);
        URL.revokeObjectURL(url);
    }
});

haystackText.addEventListener("input", function (ev) {
    // Automatically search if a valid regex module exists
    if (currentModule !== null) {
        performSearch();
    } else {
        searchResultDiv.textContent = ""; // Clear result if no valid regex
    }
});

searchButton.addEventListener("click", performSearch);
