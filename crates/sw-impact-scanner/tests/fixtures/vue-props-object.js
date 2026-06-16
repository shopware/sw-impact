import template from './props-object.html.twig';

export default {
    template,

    props: {
        title: String,
        count: Number,
        disabled: Boolean,
        items: Array,
        user: Object,
        callback: Function,
        createdAt: Date,

        special: {
            type: String,
            required: true,
        },

        always: {
            type: Object,
            default: () => ({}),
        },

        multi: [String, Number],

        multiObj: {
            type: [String, Number],
        },
    },
};
