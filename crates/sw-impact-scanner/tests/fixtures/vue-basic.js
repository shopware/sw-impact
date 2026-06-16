import template from './basic-vue.html.twig';

export default {
    template,

    emits: [
        'block-duplicate',
        'block-delete',
    ],

    props: {
        block: {
            type: Object,
            required: true,
        },

        removable: {
            type: Boolean,
            required: false,
            default() {
                return false;
            },
        },

        duplicable: {
            type: Boolean,
            required: false,
            default() {
                return true;
            },
        },
    },

    computed: {
        isLoading() {
            return this.block !== null;
        },
    },

    methods: {
        onBlockDuplicate() {
            this.$emit('block-duplicate', this.block);
        },

        onBlockDelete() {
            this.$emit('block-delete', this.block);
        },
    },
};
