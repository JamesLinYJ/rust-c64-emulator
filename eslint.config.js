import eslint from '@eslint/js';
import reactHooks from 'eslint-plugin-react-hooks';
import reactRefresh from 'eslint-plugin-react-refresh';
import globals from 'globals';
import tseslint from 'typescript-eslint';

const typedFiles = ['src/**/*.{ts,tsx}', 'tests/**/*.{ts,tsx}', 'tools/**/*.ts', 'vite.config.ts'];
const typedConfigs = [
  ...tseslint.configs.recommendedTypeChecked,
  ...tseslint.configs.stylisticTypeChecked,
].map((config) => ({ ...config, files: typedFiles }));

export default tseslint.config(
  {
    ignores: ['dist/**', 'coverage/**', 'generated/**', 'legacy/**', 'js/**', 'target/**'],
  },
  eslint.configs.recommended,
  ...typedConfigs,
  {
    files: typedFiles,
    languageOptions: {
      globals: {
        ...globals.browser,
        ...globals.node,
      },
      parserOptions: {
        projectService: true,
        tsconfigRootDir: import.meta.dirname,
      },
    },
    plugins: {
      'react-hooks': reactHooks,
      'react-refresh': reactRefresh,
    },
    rules: {
      ...reactHooks.configs.flat.recommended.rules,
      '@typescript-eslint/consistent-type-definitions': ['error', 'interface'],
      '@typescript-eslint/no-confusing-void-expression': 'off',
      '@typescript-eslint/no-explicit-any': 'error',
      '@typescript-eslint/no-non-null-assertion': 'off',
    },
  },
  {
    files: ['src/**/*.tsx'],
    rules: {
      'react-refresh/only-export-components': ['error', { allowConstantExport: true }],
    },
  },
);
