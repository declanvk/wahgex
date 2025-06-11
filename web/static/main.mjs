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

        let compileResult;
        try {
            // The compile function now returns the CompileResult object
            compileResult = compile(pattern);
        } catch (err) {
            if (typeof err === "string") {
                throw Error(err);
            } else {
                throw err;
            }
        }

        // Extract the wasm_bytes from the CompileResult
        const moduleBytes = compileResult.wasm_bytes;
        const { module, instance } = await WebAssembly.instantiate(moduleBytes);

        // Pass the compileResult object to the constructor
        return new RegexModule(pattern, compileResult, module, instance);
    }

    constructor(pattern, compileResult, module, instance) {
        this.pattern = pattern;
        this.compileResult = compileResult;
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

const searchButton = document.getElementById("searchButton");
assert(searchButton !== null, "expected search button element present");

const downloadWasmButton = document.getElementById("downloadWasmButton");
assert(
    downloadWasmButton !== null,
    "expected download WASM button element present",
);

const shareRegexButton = document.getElementById("shareRegexButton"); // Get share button
assert(
    shareRegexButton !== null,
    "expected share regex button element present",
);

const regexInput = document.getElementById("regexInput");
assert(regexInput !== null, "expected regex input element present");

const haystackText = document.getElementById("haystackText");
assert(haystackText !== null, "expected haystack text area element present");

const regexErrorDiv = document.getElementById("regexError");
assert(regexErrorDiv !== null, "expected regex error present");

const searchResultDiv = document.getElementById("searchResult");
assert(searchResultDiv !== null, "expected search result present");

// Get the new div for statistics
const statsOutputDiv = document.getElementById("statsOutput");
assert(statsOutputDiv !== null, "expected stats output present");

// Add a div for share feedback
const shareFeedbackDiv = document.getElementById("shareFeedback");
assert(statsOutputDiv !== null, "expected share feedback output present");

function clearMessages() {
    regexErrorDiv.textContent = "";
    searchResultDiv.textContent = "";
    statsOutputDiv.textContent = ""; // Clear stats
    shareFeedbackDiv.textContent = ""; // Clear share feedback
}

// Helper function to toggle button states and clear module data
function resetModuleState() {
    currentModule = null;
    searchButton.disabled = true;
    downloadWasmButton.disabled = true;
    shareRegexButton.disabled = true; // Disable share button
    statsOutputDiv.textContent = ""; // Clear stats on reset
    shareFeedbackDiv.textContent = ""; // Clear share feedback
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

// New function to display statistics
function displayStats(compileResult) {
    if (!compileResult) {
        statsOutputDiv.textContent = "";
        return;
    }

    let statsHtml = "<h2>WASM Module Statistics</h2><ul>";
    statsHtml += `<li><strong>Module size:</strong> ${compileResult.module_size} bytes</li>`;
    statsHtml += `<li><strong>States:</strong> ${compileResult.states}</li>`;
    statsHtml += `<li><strong>Pattern length:</strong> ${compileResult.pattern_len}</li>`;
    statsHtml += `<li><strong>Has capture:</strong> ${compileResult.has_capture}</li>`;
    statsHtml += `<li><strong>Has empty:</strong> ${compileResult.has_empty}</li>`;
    statsHtml += `<li><strong>Is UTF8:</strong> ${compileResult.is_utf8}</li>`;
    statsHtml += `<li><strong>Is reverse:</strong> ${compileResult.is_reverse}</li>`;
    statsHtml += `<li><strong>Lookahead any:</strong> ${compileResult.lookset_any}</li>`;
    statsHtml += `<li><strong>Lookahead prefix any:</strong> ${compileResult.lookset_prefix_any}</li>`;
    statsHtml += "</ul>";

    statsOutputDiv.innerHTML = statsHtml;
}

// Refactored function to process regex input
function processRegexInput(value) {
    clearMessages();
    if (value === null || !(typeof value === "string") || value.length === 0) {
        resetModuleState();
        clearMessages();
        return;
    }

    RegexModule.create(value)
        .then((module) => {
            currentModule = module;
            searchButton.disabled = false;
            downloadWasmButton.disabled = false;
            shareRegexButton.disabled = false; // Enable share button
            displayStats(currentModule.compileResult); // Display stats after successful compilation

            // Automatically search if haystack is not empty after successful compilation
            if (haystackText.value.length > 0) {
                performSearch();
            }
        })
        .catch((err) => {
            resetModuleState();
            clearMessages();
            regexErrorDiv.textContent = `Compilation error: ${err.message}`;
        });
}

// Event listener for the regex input field (now calls the refactored function)
regexInput.addEventListener(
    "input",
    debounce(function (ev) {
        processRegexInput(regexInput.value);
    }, 500),
);

// Event listener for the share button
shareRegexButton.addEventListener("click", async function () {
    if (currentModule && currentModule.pattern) {
        const pattern = currentModule.pattern;
        const encodedPattern = encodeURIComponent(pattern);
        const shareUrl = `${window.location.origin}${window.location.pathname}?regex=${encodedPattern}`;

        try {
            await navigator.clipboard.writeText(shareUrl);
        } catch (err) {
            shareFeedbackDiv.textContent = "Failed to copy link.";
            shareFeedbackDiv.style.color = "red";
            console.error("Failed to copy: ", err);
        }
    }
});

downloadWasmButton.addEventListener("click", async function () {
    if (currentModule && currentModule.compileResult && currentModule.pattern) {
        const blob = new Blob([currentModule.compileResult.wasm_bytes], {
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
    if (currentModule !== null) {
        performSearch();
    } else {
        searchResultDiv.textContent = "";
    }
});

searchButton.addEventListener("click", performSearch);

const urlParams = new URLSearchParams(window.location.search);
const regexFromUrl = urlParams.get("regex");

if (regexFromUrl) {
    try {
        const decodedRegex = decodeURIComponent(regexFromUrl);
        regexInput.value = decodedRegex;
        processRegexInput(decodedRegex);
    } catch (e) {
        console.error("Failed to decode or process regex from URL:", e);
        regexErrorDiv.textContent = `Error loading regex from URL: Invalid format.`;
    }
}
