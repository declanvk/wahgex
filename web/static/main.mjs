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

function getElementById(id) {
    const element = document.getElementById(id);
    if (element === null) {
        throw new Error(`Expected element with id [${id}] to be present`);
    }
    return element;
}

let currentModule = null;

const searchButton = getElementById("searchButton");
const downloadWasmButton = getElementById("downloadWasmButton");
const shareRegexButton = getElementById("shareRegexButton");
const regexInput = getElementById("regexInput");
const haystackText = getElementById("haystackText");
const regexErrorOutput = getElementById("regexError");
const searchResultOutput = getElementById("searchResult");
const statsOutput = getElementById("statsOutput");
const statsOutputList = getElementById("statsOutputList");
const watOutput = getElementById("watOutput");
const watOutputPre = getElementById("watOutputPre");
const shareRegexOutput = getElementById("shareRegex");

function clearMessages() {
    regexErrorOutput.textContent = "";
    searchResultOutput.textContent = "";
    statsOutput.style.display = "none";
    statsOutputList.innerHTML = "";
    watOutput.style.display = "none";
    watOutputPre.textContent = "";
    shareRegexOutput.textContent = "";
}

// Helper function to toggle button states and clear module data
function resetModuleState() {
    currentModule = null;
    searchButton.disabled = true;
    downloadWasmButton.disabled = true;
    shareRegexButton.disabled = true;
    statsOutput.style.display = "none";
    statsOutputList.innerHTML = "";
    watOutput.style.display = "none";
    watOutputPre.textContent = "";
    shareRegexOutput.textContent = "";
}

function performSearch() {
    searchResultOutput.textContent = "";
    if (currentModule === null) {
        return;
    }

    const haystack = haystackText.value;
    if (haystack === null || !(typeof haystack === "string")) {
        searchResultOutput.textContent = "";
        return;
    }

    const result = currentModule.isMatch(haystack);
    searchResultOutput.textContent = result
        ? "Match found!"
        : "No match found.";
}

function displayStats(compileResult) {
    // Clear previous content
    statsOutputList.innerHTML = "";
    statsOutput.style.display = "none";

    if (!compileResult) {
        return;
    }

    // Undo the `display: none;` style override
    statsOutput.style.display = "";

    const createStatItem = (label, value) => {
        const li = document.createElement("li");
        li.innerHTML = `<strong>${label}:</strong> ${value}`;
        return li;
    };

    statsOutputList.appendChild(
        createStatItem("Module size", `${compileResult.module_size} bytes`),
    );
    statsOutputList.appendChild(createStatItem("States", compileResult.states));
    statsOutputList.appendChild(
        createStatItem("Pattern length", compileResult.pattern_len),
    );
    statsOutputList.appendChild(
        createStatItem("Has capture", compileResult.has_capture),
    );
    statsOutputList.appendChild(
        createStatItem("Has empty", compileResult.has_empty),
    );
    statsOutputList.appendChild(
        createStatItem("Is UTF8", compileResult.is_utf8),
    );
    statsOutputList.appendChild(
        createStatItem("Is reverse", compileResult.is_reverse),
    );
    statsOutputList.appendChild(
        createStatItem("Lookahead any", compileResult.lookset_any),
    );
    statsOutputList.appendChild(
        createStatItem(
            "Lookahead prefix any",
            compileResult.lookset_prefix_any,
        ),
    );
}

function displayWat(compileResult) {
    // Clear previous content
    watOutputPre.textContent = "";
    watOutput.style.display = "none"; // Hide by default

    if (!compileResult) {
        return;
    }

    // Undo the `display: none;` style override
    watOutput.style.display = "";
    watOutputPre.textContent = compileResult.wat_string;
}

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
            shareRegexButton.disabled = false;
            displayStats(currentModule.compileResult); // Display stats after successful compilation
            displayWat(currentModule.compileResult); // Display WAT (placeholder) after successful compilation

            // Automatically search if haystack is not empty after successful compilation
            if (haystackText.value.length > 0) {
                performSearch();
            }
        })
        .catch((err) => {
            resetModuleState();
            clearMessages();
            regexErrorOutput.textContent = `Compilation error: ${err.message}`;
        });
}

// Event listener for the regex input field
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
        const urlParams = new URLSearchParams(window.location.search);
        urlParams.set("regex", encodedPattern);
        const shareUrl = `${window.location.origin}${window.location.pathname}?${urlParams.toString()}`;

        try {
            await navigator.clipboard.writeText(shareUrl);
            shareRegexOutput.textContent = "Copied link to clipboard!";
        } catch (err) {
            shareRegexOutput.textContent = "Failed to copy link.";
            shareRegexOutput.style.color = "red";
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
        searchResultOutput.textContent = "";
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
        regexErrorOutput.textContent = `Error loading regex from URL: Invalid format.`;
    }
}
