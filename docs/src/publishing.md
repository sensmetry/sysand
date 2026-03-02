# Publishing a package to Sysand Index

This guide will instruct how to submit your package to Sysand Package Index
manually. Sensmetry is currently working on automating this process.

## Manual submission

1. **Produce your package**

   Sysand CLI at the moment can only package textual SysML v2 or KerML files
   into `.kpar` project interchange archives. Therefore, the Sysand Package
   Index currently only accepts projects developed in SysML v2 or KerML textual
   files.

2. **Add project metadata**

   Project metadata relevant here is stored in `.project.json`. It can
   either be edited using Sysand (recommended) or by using any text editor.
   `sysand info` is used to modify the information.
   - **Name** of the project: Make it short and sweet. Avoid having spaces in
     the name if possible.

     Set it with `sysand info name --set <NAME>`

   - **Description**: Couple of sentences describing what this project can be
     used for. Keep it succinct.

     Set it with `sysand info description --set <DESCRIPTION>`

   - **Version**: A version number of the project you're submitting. Shall
     follow the Semantic Versioning schema.

     Set it with `sysand info version --set <VERSION>`

   - **License**: Which license this project is licensed under, e.g. MIT,
     Apache, GPL, any other. Ideally the license has an identifier from SPDX. If no
     license is given here, we will assume that the package is licensed under the
     MIT license.

     Set it with `sysand info license --set <SPDX LICENSE EXPRESSION>`

   - **Maintainer**: A list of names of main maintainers (individuals or
     companies), preferably with email address(es), of the project.

     Add a maintainer to the list with `sysand info maintainer --add <MAINTAINER>`

   - **Website** (optional): A URL to where people can find more information
     about the project. Can be a homepage, or a GitHub/GitLab repository.

     Set it with `sysand info website --set <WEBSITE>`

   - **Topic**: A list of topics relevant to the project. These will be treated
     as keywords. In the future, users will be able to search/filter the package
     index using topics.

     Add a topic to the list with `sysand info topic --add <TOPIC>`

   - **Usage**: A list of dependencies this project depends on, with links
     to them if the dependencies do not exist on the Sysand package index yet. By
     default your project will most likely depend on SysML v2 standard libraries.
     **Important: To be able to publish your package to the index, all of its
     dependencies must be publicly available!**

     Add usage to the list with `sysand add <IRI>`

3. **(Optional) Package the project into `.kpar`**

   Once you have filled out the metadata requested above, you can use Sysand
   CLI to package the project into one neat `.kpar` file.

   More information on how to do that can be found in the
   [Tutorial](getting_started/tutorial.md). The short version, assuming all
   `.sysml`/`.kerml` files are already included in the project, is to run this
   command inside your project:

   ```sh
   sysand build
   ```

   The built `.kpar` file will be in the `output/` folder.

4. **Send the project to Sysand team**

   When you have the project and the answers to the
   questionnaire prepared, please send them as an email to
   [sysand@sensmetry.com](mailto:sysand@sensmetry.com). The Sysand team
   will review the project and the metadata, package everything into a .kpar
   package, and upload it to the Index.

   Do not forget to attach the required `.sysml`/`.kerml` (or `.kpar`) files to
   the email!

> [!important]
> **Terms of publishing**
>
> By submitting your package to the Sysand Package Index, you confirm that:
>
> - You agree not to submit packages containing malicious code, inappropriate
>   content, or content that infringes on third-party rights.
> - You (co-)own the rights to the package and have the rights to redistribute
>   it.
> - The package does not contain any proprietary or confidential data that
>   cannot be shared publicly.
> - You acknowledge that you are responsible for maintaining accurate package
>   information and metadata.
> - You give the right to Sensmetry to redistribute the package publicly through
>   the Sysand Package Index.
> - You consent to Sensmetry running basic validations on the package using the
>   Syside tool and making the validation results public.
> - Accept that Sensmetry reserves the right to modify these terms and the
>   service without prior notice.
