# Template Updates

This project was generated from the [claylo-rs](https://github.com/claylo/claylo-rs) Copier template.
You can pull in template improvements while preserving your customizations.

## Updating the Template

```bash
# Preview changes without applying
copier update --pretend

# Apply template updates
copier update
```

Copier will:

1. Re-render template files with your stored answers (from `.repo.yml`)
2. Show a diff for any conflicts
3. Let you accept, reject, or manually merge changes

## Managing Copyright Year

The `copyright_year` is computed once when the project is created and stored in `.repo.yml`.
This prevents it from changing unexpectedly on template updates.

### Updating to a Year Range

If you created the project in 2026 and want to update the copyright to reflect ongoing development:

1. Edit `.repo.yml`:

   ```yaml
   # Before
   copyright_year: '2026'

   # After
   copyright_year: '2026-2028'
   ```

2. Run template update to regenerate license files:

   ```bash
   copier update
   ```

   Or manually edit `LICENSE-MIT`.

### Why It Works This Way

The template uses Copier's "locked value" pattern:

```yaml
copyright_year:
  default: "2026"
  when: false
```

- **First run**: Computes current year via `strftime`
- **Subsequent updates**: Uses value from `.repo.yml`, ignoring current date

This ensures a project created in 2026 doesn't suddenly show "2030" after a template update in 2030.

## Other Locked Values

The following values are also computed once and stored:

| Value | Description | How to Change |
|-------|-------------|---------------|
| `copyright_year` | Year(s) in license | Edit `.repo.yml` |

## Skipping Template Updates for Specific Files

If you've heavily customized a file and want to exclude it from updates:

```bash
copier update --skip README.md --skip .github/workflows/ci.yml
```

## Checking Template Version

Your current template version is stored in `.repo.yml`:

```yaml
_commit: abc1234  # Git commit of template when last applied
_src_path: https://github.com/claylo/claylo-rs
```
