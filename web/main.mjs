import init, { compile } from './pkg/wahgex_web.js';

await init();

// Copied from https://codepen.io/wonism/pen/dXgrxE
function debounce(cb, interval, immediate) {
    var timeout;

    return function () {
        var context = this, args = arguments;
        var later = function () {
            timeout = null;
            if (!immediate) cb.apply(context, args);
        };

        var callNow = immediate && !timeout;

        clearTimeout(timeout);
        timeout = setTimeout(later, interval);

        if (callNow) cb.apply(context, args);
    };
};

class RegexModule {
    static async create(pattern) {
        if (pattern == null || !(typeof pattern === 'string')) {
            return null;
        }

        const moduleBytes = compile(pattern);
        const { module, instance } = await WebAssembly.instantiate(moduleBytes);

        return new RegexModule(pattern, moduleBytes, module, instance);
    }

    constructor(pattern, moduleBytes, module, instance) {
        this.pattern = pattern;
        this.moduleBytes = moduleBytes;
        this.module = module;
        this.instance = instance;
        this.encoder = new TextEncoder('utf-8');
    }

    isMatch(haystack) {
        if (haystack === null || !(typeof haystack === 'string')) {
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
        const result = this.instance.exports.is_match(anchored, anchoredPattern, spanStart, spanEnd, haystackLen);
        return Boolean(result);
    }

}

function assert(condition, message) {
    if (!condition) {
        throw new Error(message || "Assertion failed");
    }
}

let currentModule = null;

const searchButton = document.getElementById('searchButton');
assert(searchButton !== null, "expected search button element present");

const regexInput = document.getElementById('regexInput');
assert(regexInput !== null, "expected regex input element present");

const compileButton = document.getElementById('compileButton');
assert(compileButton !== null, "expected compile button element present");

const haystackText = document.getElementById('haystackText');
assert(haystackText !== null, "expected haystack text area element present");

regexInput.addEventListener('input', debounce(function (ev) {
    const value = regexInput.value;
    if (value === null || !(typeof value === 'string') || value.length === 0) {
        currentModule = null;
        searchButton.disabled = true;
        return;
    }

    RegexModule.create(value).then(module => {
        currentModule = module;
        searchButton.disabled = false;
    }).catch(err => {
        regexInput.setCustomValidity(err.message);
    });
}, 500));


compileButton.addEventListener('click', function (ev) {
    if (regexInput === null) {
        return;
    }

    const value = regexInput.value;
    if (value === null || !(typeof value === 'string') || value.length === 0) {
        currentModule = null;
        searchButton.disabled = true;
        return;
    }

    RegexModule.create(value).then(module => {
        currentModule = module;
        searchButton.disabled = false;
    }).catch(err => {
        regexInput.setCustomValidity(err.message);
    });
});

searchButton.addEventListener('click', function (ev) {
    if (currentModule === null) {
        return;
    }

    const haystack = haystackText.value;
    if (haystack === null || !(typeof haystack === 'string') || haystack.length === 0) {
        return;
    }

    const result = currentModule.isMatch(haystack);
    console.log(result);
});
